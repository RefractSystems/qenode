#![allow(unused_variables)]
#![allow(clippy::all)]
#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::needless_return,
    clippy::manual_range_contains,
    clippy::single_component_path_imports,
    clippy::len_zero,
    clippy::while_immutable_condition
)]

use core::ffi::{c_char, c_int, c_uint, c_void};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::Arc;
use virtmcu_qom::error::Error;
use virtmcu_qom::irq::{qemu_irq, qemu_set_irq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_get_connected_irq, sysbus_init_mmio, DeviceClass, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::BqlGuarded;
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    device_class_set_props, error_setg,
};
use virtmcu_zenoh::SafeSubscriber;
use zenoh::Session;
use zenoh::Wait;

#[repr(C)]
pub struct ZenohUiQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    /* Properties */
    pub node_id: u32,
    pub router: *mut c_char,

    /* Registers */
    pub active_led_id: u32,
    pub active_btn_id: u32,

    /* Rust state */
    pub rust_state: *mut ZenohUiState,
}

pub struct ZenohUiState {
    session: Session,
    node_id: u32,
    buttons: BqlGuarded<HashMap<u32, ButtonState>>,
}

struct ButtonState {
    irq: qemu_irq,
    subscriber: Option<SafeSubscriber>,
    pressed: bool,
}

const REG_LED_ID: u64 = 0x00;
const REG_LED_STATE: u64 = 0x04;
const REG_BTN_ID: u64 = 0x10;
const REG_BTN_STATE: u64 = 0x14;

unsafe extern "C" fn zenoh_ui_read(opaque: *mut c_void, addr: u64, _size: c_uint) -> u64 {
    let s = &mut *(opaque as *mut ZenohUiQEMU);
    if addr == REG_LED_ID {
        return u64::from(s.active_led_id);
    }
    if addr == REG_BTN_ID {
        return u64::from(s.active_btn_id);
    }
    if addr == REG_BTN_STATE {
        if s.rust_state.is_null() {
            return 0;
        }
        return u64::from(zenoh_ui_get_button(&*s.rust_state, s.active_btn_id));
    }
    0
}

unsafe extern "C" fn zenoh_ui_write(opaque: *mut c_void, addr: u64, val: u64, _size: c_uint) {
    let s = &mut *(opaque as *mut ZenohUiQEMU);
    if addr == REG_LED_ID {
        s.active_led_id = val as u32;
    } else if addr == REG_LED_STATE {
        if !s.rust_state.is_null() {
            zenoh_ui_set_led(&*s.rust_state, s.active_led_id, val != 0);
        }
    } else if addr == REG_BTN_ID {
        s.active_btn_id = val as u32;
        let irq = sysbus_get_connected_irq(opaque as *mut SysBusDevice, s.active_btn_id as c_int);
        if !s.rust_state.is_null() {
            zenoh_ui_ensure_button(&*s.rust_state, s.active_btn_id, irq);
        }
    }
}

static ZENOH_UI_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(zenoh_ui_read),
    write: Some(zenoh_ui_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 4,
        max_access_size: 4,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 0,
        max_access_size: 0,
        unaligned: false,
        _padding: [0; 7],
    },
};

unsafe extern "C" fn zenoh_ui_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut ZenohUiQEMU);

    memory_region_init_io(
        &raw mut s.mmio,
        dev as *mut Object,
        &raw const ZENOH_UI_OPS,
        dev,
        c"zenoh-ui".as_ptr(),
        0x100,
    );
    sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    s.rust_state = zenoh_ui_init_internal(s.node_id, router_ptr);
    if s.rust_state.is_null() {
        error_setg!(errp, "Failed to initialize Rust Zenoh UI");
        return;
    }
}

unsafe extern "C" fn zenoh_ui_instance_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut ZenohUiQEMU);
    if !s.rust_state.is_null() {
        let state = Box::from_raw(s.rust_state);
        {
            let mut btns = state.buttons.get_mut();
            for (_, btn) in btns.iter_mut() {
                btn.subscriber.take();
            }
        }
        drop(state);
        s.rust_state = ptr::null_mut();
    }
}

define_properties!(
    ZENOH_UI_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), ZenohUiQEMU, node_id, 0),
        define_prop_string!(c"router".as_ptr(), ZenohUiQEMU, router),
    ]
);

unsafe extern "C" fn zenoh_ui_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(zenoh_ui_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, ZENOH_UI_PROPERTIES);
}

static ZENOH_UI_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-ui".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<ZenohUiQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_ui_instance_finalize),
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_ui_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(zenoh_ui_type_init, ZENOH_UI_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_ui_init_internal(node_id: u32, router: *const c_char) -> *mut ZenohUiState {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    Box::into_raw(Box::new(ZenohUiState {
        session,
        node_id,
        buttons: BqlGuarded::new(HashMap::new()),
    }))
}

fn zenoh_ui_set_led(state: &ZenohUiState, led_id: u32, on: bool) {
    let topic = format!("sim/ui/{}/led/{}", state.node_id, led_id);
    let payload = if on { vec![1u8] } else { vec![0u8] };
    let _ = state.session.put(topic, payload).wait();
}

fn zenoh_ui_get_button(state: &ZenohUiState, btn_id: u32) -> bool {
    let btns = state.buttons.get();
    btns.get(&btn_id).is_some_and(|b| b.pressed)
}

fn zenoh_ui_ensure_button(state: &ZenohUiState, btn_id: u32, irq: qemu_irq) {
    let mut btns = state.buttons.get_mut();
    if btns.contains_key(&btn_id) {
        return;
    }

    let topic = format!("sim/ui/{}/button/{}", state.node_id, btn_id);
    let irq_ptr = irq as usize;

    let subscriber = SafeSubscriber::new(&state.session, &topic, move |sample| {
        let payload = sample.payload();
        if payload.len() < 1 {
            return;
        }
        let val = payload.to_bytes()[0] != 0;

        unsafe {
            qemu_set_irq(irq_ptr as qemu_irq, i32::from(val));
        }
    })
    .ok();

    btns.insert(btn_id, ButtonState { irq, subscriber, pressed: false });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zenoh_ui_qemu_layout() {
        // QOM layout validation
        assert_eq!(
            core::mem::offset_of!(ZenohUiQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}

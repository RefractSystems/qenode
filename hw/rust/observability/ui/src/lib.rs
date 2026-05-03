use zenoh::Wait;
extern crate alloc;

use alloc::sync::Arc;
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr;
use std::collections::HashMap;
use transport_zenoh::SafeSubscriber;
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_get_connected_irq, sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::BqlGuarded;
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};
use zenoh::Session;

#[repr(C)]
pub struct ZenohUiQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    /* Properties */
    pub node_id: u32,
    pub router: *mut c_char,
    pub debug: bool,

    /* Registers */
    pub active_led_id: u32,
    pub active_btn_id: u32,

    /* Rust state */
    pub rust_state: *mut ZenohUiState,
}

pub struct ZenohUiState {
    _session: Arc<Session>,
    publisher: transport_zenoh::SafeSessionPublisher,
    node_id: u32,
    buttons: BqlGuarded<HashMap<u32, ButtonState>>,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

struct ButtonState {
    _irq: QemuIrq,
    subscriber: Option<SafeSubscriber>,
    pressed: bool,
}

const REG_LED_ID: u64 = 0x00;
const REG_LED_STATE: u64 = 0x04;
const REG_BTN_ID: u64 = 0x10;
const REG_BTN_STATE: u64 = 0x14;

/// # Safety
/// This function is called by QEMU. opaque must be a valid pointer to ZenohUiQEMU.
#[no_mangle]
pub unsafe extern "C" fn ui_read(opaque: *mut c_void, addr: u64, _size: c_uint) -> u64 {
    // SAFETY: opaque is a valid pointer to ZenohUiQEMU provided by QEMU.
    let s = unsafe { &mut *(opaque as *mut ZenohUiQEMU) };
    if s.debug {
        virtmcu_qom::sim_warn!("ui_read: addr=0x{:x}", addr);
    }
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
        // SAFETY: rust_state is non-null and owned by the device.
        return u64::from(ui_get_button(unsafe { &*s.rust_state }, s.active_btn_id));
    }
    0
}

/// # Safety
/// This function is called by QEMU. opaque must be a valid pointer to ZenohUiQEMU.
#[no_mangle]
pub unsafe extern "C" fn ui_write(opaque: *mut c_void, addr: u64, val: u64, _size: c_uint) {
    // SAFETY: opaque is a valid pointer to ZenohUiQEMU provided by QEMU.
    let s = unsafe { &mut *(opaque as *mut ZenohUiQEMU) };
    if s.debug {
        virtmcu_qom::sim_warn!("ui_write: addr=0x{:x} val=0x{:x}", addr, val);
    }
    if addr == REG_LED_ID {
        s.active_led_id = val as u32;
    } else if addr == REG_LED_STATE {
        if !s.rust_state.is_null() {
            // SAFETY: rust_state is non-null and owned by the device.
            ui_set_led(unsafe { &*s.rust_state }, s.active_led_id, val != 0);
        }
    } else if addr == REG_BTN_ID {
        s.active_btn_id = val as u32;
        // SAFETY: opaque is a valid pointer to SysBusDevice.
        let irq = unsafe {
            sysbus_get_connected_irq(opaque as *mut SysBusDevice, s.active_btn_id as c_int)
        };
        if !s.rust_state.is_null() {
            // SAFETY: rust_state is non-null and owned by the device.
            ui_ensure_button(unsafe { &*s.rust_state }, s.active_btn_id, irq);
        }
    }
}

static ZENOH_UI_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(ui_read),
    write: Some(ui_write),
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

/// # Safety
/// This function is called by QEMU to realize the device. dev must be a valid pointer to ZenohUiQEMU.
#[no_mangle]
pub unsafe extern "C" fn ui_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    // SAFETY: dev is a valid pointer to ZenohUiQEMU provided by QEMU.
    let s = unsafe { &mut *(dev as *mut ZenohUiQEMU) };

    // SAFETY: s->mmio is initialized by QEMU MemoryRegion API.
    unsafe {
        memory_region_init_io(
            &raw mut s.mmio,
            dev as *mut Object,
            &raw const ZENOH_UI_OPS,
            dev,
            c"ui".as_ptr(),
            0x100,
        );
        sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);
    }

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    s.rust_state = ui_init_internal(s.node_id, router_ptr);
    if s.rust_state.is_null() {
        error_setg!(errp, "Failed to initialize Rust Zenoh UI");
    }
}

/// # Safety
/// This function is called by QEMU when finalizing the device. obj must be a valid pointer to ZenohUiQEMU.
#[no_mangle]
pub unsafe extern "C" fn ui_instance_finalize(obj: *mut Object) {
    // SAFETY: obj is a valid pointer to ZenohUiQEMU provided by QEMU.
    let s = unsafe { &mut *(obj as *mut ZenohUiQEMU) };
    if !s.rust_state.is_null() {
        // SAFETY: rust_state was allocated via Box::into_raw and is non-null.
        let state = unsafe { Box::from_raw(s.rust_state) };
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
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), ZenohUiQEMU, debug, false),
    ]
);

/// # Safety
/// This function is called by QEMU to initialize the class. klass must be a valid pointer to ObjectClass.
#[no_mangle]
pub unsafe extern "C" fn ui_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    // SAFETY: dc is a valid DeviceClass pointer.
    unsafe {
        (*dc).realize = Some(ui_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::qdev::device_class_set_props_n(dc, ZENOH_UI_PROPERTIES.as_ptr(), 3);
}

#[used]
static ZENOH_UI_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"ui".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<ZenohUiQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(ui_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(ui_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(ZENOH_UI_TYPE_INIT, ZENOH_UI_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn ui_init_internal(node_id: u32, router: *const c_char) -> *mut ZenohUiState {
    // SAFETY: get_or_init_session is safe if router is valid or null.
    // Safety: router validity is guaranteed by the caller.
    let session = unsafe {
        match transport_zenoh::get_or_init_session(router) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    let liveliness =
        session.liveliness().declare_token(format!("sim/ui/liveliness/{node_id}")).wait().ok();
    Box::into_raw(Box::new(ZenohUiState {
        _liveliness: liveliness,
        publisher: transport_zenoh::SafeSessionPublisher::new(Arc::clone(&session)),
        _session: session,
        node_id,
        buttons: BqlGuarded::new(HashMap::new()),
    }))
}

fn ui_set_led(state: &ZenohUiState, led_id: u32, on: bool) {
    let topic = format!("sim/ui/{}/led/{}", state.node_id, led_id);
    let payload = if on { vec![1u8] } else { vec![0u8] };
    state.publisher.send(topic, payload);
}

fn ui_get_button(state: &ZenohUiState, btn_id: u32) -> bool {
    let btns = state.buttons.get();
    btns.get(&btn_id).is_some_and(|b| b.pressed)
}

fn ui_ensure_button(state: &ZenohUiState, btn_id: u32, irq: QemuIrq) {
    let mut btns = state.buttons.get_mut();
    if btns.contains_key(&btn_id) {
        return;
    }

    let topic = format!("sim/ui/{}/button/{}", state.node_id, btn_id);
    let irq_ptr = irq as usize;

    let subscriber = SafeSubscriber::new(&state._session, &topic, move |sample| {
        let payload = sample.payload();
        if payload.is_empty() {
            return;
        }
        let val = payload.to_bytes()[0] != 0;

        // SAFETY: irq_ptr is a valid QemuIrq passed during initialization.
        unsafe {
            qemu_set_irq(irq_ptr as QemuIrq, i32::from(val));
        }
    })
    .ok();

    btns.insert(btn_id, ButtonState { _irq: irq, subscriber, pressed: false });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_qemu_layout() {
        // QOM layout validation
        assert_eq!(
            core::mem::offset_of!(ZenohUiQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}

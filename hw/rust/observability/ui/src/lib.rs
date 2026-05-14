#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
unsafe extern "C" fn allow_set_link(
    _obj: *mut virtmcu_qom::qom::Object,
    _name: *const core::ffi::c_char,
    _val: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut virtmcu_qom::error::Error,
) {
}
extern crate alloc;

use alloc::sync::Arc;
use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use std::collections::HashMap;
use virtmcu_qom::irq::{qemu_set_irq, QemuIrq};
use virtmcu_qom::memory::{memory_region_init_io, MemoryRegion};
use virtmcu_qom::qdev::{sysbus_get_connected_irq, sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::BqlGuarded;
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
pub struct ZenohUiQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    /* Properties */
    pub node_id: u32,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub debug: bool,

    /* Links */
    pub transport_hub: *mut Object,

    /* Registers */
    pub active_led_id: u32,
    pub active_btn_id: u32,

    /* Rust state */
    pub rust_state: *mut ZenohUiState,
}

const _: () = assert!(core::mem::offset_of!(ZenohUiQEMU, parent_obj) == 0);
const _: () = assert!(core::mem::size_of::<ZenohUiQEMU>() == 1152);

pub struct ZenohUiState {
    parent_ptr: *mut ZenohUiQEMU,
    transport: Arc<dyn virtmcu_api::DataTransport>,
    node_id: u32,
    buttons: BqlGuarded<HashMap<u32, ButtonState>>,
    cond: virtmcu_qom::sync::Condvar,
    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    wait_mutex: virtmcu_qom::sync::Mutex<()>,
    pub _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
}

struct ButtonState {
    _irq: QemuIrq,
    pressed: bool,
}

const REG_LED_ID: u64 = 0x00;
const REG_LED_STATE: u64 = 0x04;
const REG_BTN_ID: u64 = 0x10;
const REG_BTN_STATE: u64 = 0x14;

impl virtmcu_qom::device::MmioDevice for ZenohUiState {
    fn read(&self, addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let s = unsafe { &mut *self.parent_ptr };
        if s.debug {
            virtmcu_qom::sim_debug!("ui_read: addr=0x{:x}", addr);
        }
        if addr == REG_LED_ID {
            return virtmcu_qom::device::MmioResult::Ready(u64::from(s.active_led_id));
        }
        if addr == REG_BTN_ID {
            return virtmcu_qom::device::MmioResult::Ready(u64::from(s.active_btn_id));
        }
        if addr == REG_BTN_STATE {
            return virtmcu_qom::device::MmioResult::Ready(u64::from(ui_get_button(
                self,
                s.active_btn_id,
            )));
        }
        virtmcu_qom::device::MmioResult::Ready(0)
    }

    fn write(&self, addr: u64, val: u64, _size: u32) {
        let s = unsafe { &mut *self.parent_ptr };
        if addr == REG_LED_ID {
            s.active_led_id = u32::try_from(val).expect("Invalid data format");
        } else if addr == REG_LED_STATE {
            ui_set_led(self, s.active_led_id, val != 0);
        } else if addr == REG_BTN_ID {
            s.active_btn_id = u32::try_from(val).expect("Invalid data format");
            let irq = unsafe {
                sysbus_get_connected_irq(
                    self.parent_ptr as *mut SysBusDevice,
                    s.active_btn_id as c_int,
                )
            };
            ui_ensure_button(self, s.active_btn_id, irq);
        } else {
            unreachable!("ui_write: unhandled offset 0x{:x} val=0x{:x}", addr, val);
        }
    }

    fn condvar(&self) -> &virtmcu_qom::sync::Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Required for Condvar::wait_yielding_bql"
    fn wait_mutex(&self) -> &virtmcu_qom::sync::Mutex<()> {
        &self.wait_mutex
    }
}

/// # Safety
/// This function is called by QEMU to realize the device. dev must be a valid pointer to ZenohUiQEMU.
#[no_mangle]
pub unsafe extern "C" fn ui_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    const UI_MMIO_SIZE: u64 = 0x100;
    // SAFETY: dev is a valid pointer to ZenohUiQEMU provided by QEMU.
    let s = unsafe { &mut *(dev as *mut ZenohUiQEMU) };

    // SAFETY: s.mmio is a valid MemoryRegion, dev is a valid object.
    unsafe {
        memory_region_init_io(
            &raw mut s.mmio,
            dev as *mut Object,
            &raw const ZENOHUIQEMU_OPS,
            dev,
            c"ui".as_ptr(),
            UI_MMIO_SIZE,
        );
        sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);
    }

    if s.transport_hub.is_null() {
        error_setg!(errp, "Strict DI violation: transport_hub link is required.");
        return;
    }

    let ptr_u64 = unsafe {
        virtmcu_qom::qom::object_property_get_uint(
            s.transport_hub,
            c"transport_ptr".as_ptr(),
            errp as *mut *mut virtmcu_qom::error::Error,
        )
    };
    if ptr_u64 == 0 {
        error_setg!(errp, "Strict DI violation: failed to acquire transport from hub.");
        return;
    }
    let transport_ref =
        unsafe { &*(ptr_u64 as *const alloc::sync::Arc<dyn virtmcu_api::DataTransport>) };
    let transport = alloc::sync::Arc::clone(transport_ref);

    s.rust_state = ui_init_internal(s, s.node_id, transport);
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
            btns.clear();
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
    virtmcu_qom::device_class_set_props!(dc, ZENOH_UI_PROPERTIES);

    unsafe {
        virtmcu_qom::qom::object_class_property_add_link(
            klass,
            c"transport".as_ptr(),
            c"virtmcu-transport-hub".as_ptr(),
            core::mem::offset_of!(ZenohUiQEMU, transport_hub) as isize,
            Some(allow_set_link),
            virtmcu_qom::qom::OBJ_PROP_LINK_STRONG,
        );
    }
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

fn ui_init_internal(
    s: &mut ZenohUiQEMU,
    node_id: u32,
    transport: Arc<dyn virtmcu_api::DataTransport>,
) -> *mut ZenohUiState {
    let liveliness = transport.declare_liveliness(&format!("sim/ui/liveliness/{node_id}"));

    Box::into_raw(Box::new(ZenohUiState {
        parent_ptr: core::ptr::from_mut(s),
        _liveliness: liveliness,
        transport,
        node_id,
        buttons: BqlGuarded::new(HashMap::new()),
        cond: virtmcu_qom::sync::Condvar::new(),
        wait_mutex: virtmcu_qom::sync::Mutex::new(()),
    }))
}

fn ui_set_led(state: &ZenohUiState, led_id: u32, on: bool) {
    let topic = format!("sim/ui/{}/led/{}", state.node_id, led_id);
    let payload = if on { vec![1u8] } else { vec![0u8] };
    let _ = state.transport.publish(&topic, &payload);
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

    let sub_callback: virtmcu_api::DataCallback = Box::new(move |_topic: &str, payload: &[u8]| {
        if payload.is_empty() {
            return;
        }
        let val = payload.first().is_some_and(|&b| b != 0);

        // SAFETY: irq_ptr is a valid QemuIrq passed during initialization.
        unsafe {
            qemu_set_irq(irq_ptr as QemuIrq, i32::from(val));
        }
    });

    let _ = state.transport.subscribe(&topic, sub_callback);

    btns.insert(btn_id, ButtonState { _irq: irq, pressed: false });
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

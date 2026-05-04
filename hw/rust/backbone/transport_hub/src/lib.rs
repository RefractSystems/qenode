#![no_std] // NO_STD_EXCEPTION: Requires libc panic for aborting
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

extern crate alloc;

use alloc::sync::Arc;
use core::ffi::{c_char, c_void};
use core::ptr;
use virtmcu_qom::qdev::{SysBusDevice, SysBusDeviceClass};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{define_prop_string, define_properties};

#[repr(C)]
pub struct VirtmcuTransportHub {
    pub parent_obj: SysBusDevice,
    pub router: *mut c_char,
    pub rust_state: *mut HubState,
}

pub struct HubState {
    pub session: Option<Arc<zenoh::Session>>,
}

const _: () = assert!(core::mem::offset_of!(VirtmcuTransportHub, parent_obj) == 0);
const _: () = assert!(core::mem::size_of::<VirtmcuTransportHub>() == 824);

define_properties!(
    VIRT_HUB_PROPERTIES,
    [define_prop_string!(c"router".as_ptr(), VirtmcuTransportHub, router),]
);

unsafe extern "C" fn hub_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut VirtmcuTransportHub);

    let router_str = if s.router.is_null() { ptr::null() } else { s.router as *const c_char };

    let session = if router_str.is_null() {
        None
    } else {
        match transport_zenoh::open_session(router_str) {
            Ok(sess) => Some(Arc::new(sess)),
            Err(_) => {
                let err_msg =
                    alloc::ffi::CString::new("Failed to open transport session for hub").unwrap();
                virtmcu_qom::error::virtmcu_error_setg(
                    _errp as *mut *mut virtmcu_qom::error::Error,
                    err_msg.as_ptr(),
                );
                return;
            }
        }
    };

    let state = alloc::boxed::Box::new(HubState { session });
    s.rust_state = alloc::boxed::Box::into_raw(state);
}

unsafe extern "C" fn hub_finalize(obj: *mut Object) {
    let s = &mut *(obj as *mut VirtmcuTransportHub);
    if !s.rust_state.is_null() {
        let _ = alloc::boxed::Box::from_raw(s.rust_state);
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn hub_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = virtmcu_qom::device_class!(klass);
    unsafe {
        (*dc).realize = Some(hub_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, VIRT_HUB_PROPERTIES);
}

#[used]
static VIRT_HUB_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"virtmcu-transport-hub".as_ptr(),
    parent: virtmcu_qom::qdev::TYPE_SYS_BUS_DEVICE,
    instance_size: core::mem::size_of::<VirtmcuTransportHub>(),
    instance_align: core::mem::align_of::<VirtmcuTransportHub>(),
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(hub_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<SysBusDeviceClass>(),
    class_init: Some(hub_class_init as unsafe extern "C" fn(*mut ObjectClass, *const c_void)),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

virtmcu_qom::declare_device_type!(virtmcu_transport_hub_register_types, VIRT_HUB_TYPE_INFO);

/// Safe API for other peripherals to extract the session from the hub object.
///
/// # Safety
/// `hub_obj` must be a valid QOM Object pointer.
#[no_mangle]
pub unsafe extern "C" fn virtmcu_hub_get_session(hub_obj: *mut Object) -> *mut c_void {
    if hub_obj.is_null() {
        return ptr::null_mut();
    }

    let hub_ptr = virtmcu_qom::qom::object_dynamic_cast(hub_obj, c"virtmcu-transport-hub".as_ptr());

    if hub_ptr.is_null() {
        return ptr::null_mut(); // Not a hub
    }

    let s = &*(hub_ptr as *mut VirtmcuTransportHub);
    if s.rust_state.is_null() {
        return ptr::null_mut();
    }

    let state = &*(s.rust_state);
    match &state.session {
        Some(sess) => Arc::into_raw(Arc::clone(sess)) as *mut c_void,
        None => ptr::null_mut(),
    }
}

/// Safely drop a session extracted via `virtmcu_hub_get_session`.
///
/// # Safety
/// `sess_ptr` must be a pointer previously returned by `virtmcu_hub_get_session`.
#[no_mangle]
pub unsafe extern "C" fn virtmcu_hub_drop_session(sess_ptr: *mut c_void) {
    if !sess_ptr.is_null() {
        let _ = Arc::from_raw(sess_ptr as *const zenoh::Session);
    }
}

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
use virtmcu_qom::{define_prop_string, define_prop_uint32, define_properties};

#[repr(C)]
pub struct VirtmcuTransportHub {
    pub parent_obj: SysBusDevice,
    pub node_id: u32,
    pub router: *mut c_char,
    pub rust_state: *mut HubState,
    pub transport_ptr: u64,
}

pub struct HubState {
    pub session: Option<Arc<zenoh::Session>>,
    pub _transport: Option<alloc::boxed::Box<Arc<dyn virtmcu_api::DataTransport>>>,
}

const _: () = assert!(core::mem::offset_of!(VirtmcuTransportHub, parent_obj) == 0);
const _: () = assert!(core::mem::size_of::<VirtmcuTransportHub>() == 840);

define_properties!(
    VIRT_HUB_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuTransportHub, node_id, 0),
        define_prop_string!(c"router".as_ptr(), VirtmcuTransportHub, router),
    ]
);

unsafe extern "C" fn hub_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut VirtmcuTransportHub);
    if !s.rust_state.is_null() {
        return;
    }

    virtmcu_qom::sim_info!(
        "hub_realize started for node={}, router={:?}",
        s.node_id,
        if s.router.is_null() { "null" } else { "non-null" }
    );

    let router_str = if s.router.is_null() { ptr::null() } else { s.router as *const c_char };

    let session = if router_str.is_null() {
        None
    } else {
        match transport_zenoh::open_session(router_str) {
            Ok(sess) => Some(Arc::new(sess)),
            Err(e) => {
                // Non-fatal: log and degrade gracefully. Peripherals that require
                // a transport will detect transport_ptr == 0 and fail at realize time.
                // This allows implicit hubs (created by -global injection) to not crash
                // QEMU when the router is unreachable (e.g., under ASan or in non-networked tests).
                virtmcu_qom::sim_warn!(
                    "hub_realize: failed to open Zenoh session (node={}): {:?}",
                    s.node_id,
                    e
                );
                None
            }
        }
    };

    let transport: Option<Arc<dyn virtmcu_api::DataTransport>> = if let Some(sess) = &session {
        Some(Arc::new(transport_zenoh::ZenohDataTransport::new(Arc::clone(sess))))
    } else {
        None
    };

    let _transport = transport.map(alloc::boxed::Box::new);

    let state = alloc::boxed::Box::new(HubState { session, _transport });

    // Set transport_ptr to the address of the Arc inside the Box inside the state Box.
    if let Some(t) = &state._transport {
        s.transport_ptr = &**t as *const Arc<dyn virtmcu_api::DataTransport> as u64;
    } else {
        s.transport_ptr = 0;
    }

    s.rust_state = alloc::boxed::Box::into_raw(state);
    virtmcu_qom::sim_info!("hub_realize finished. transport_ptr={}", s.transport_ptr);
}

unsafe extern "C" fn hub_instance_init(obj: *mut Object) {
    let s = &mut *(obj as *mut VirtmcuTransportHub);
    s.transport_ptr = 0;
    virtmcu_qom::qom::object_property_add_uint64_ptr(
        obj,
        c"transport_ptr".as_ptr(),
        &s.transport_ptr,
        virtmcu_qom::qom::OBJ_PROP_FLAG_READ,
    );
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
    instance_init: Some(hub_instance_init),
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
pub unsafe extern "C" fn virtmcu_hub_get_transport(hub_obj: *mut Object) -> *mut c_void {
    if hub_obj.is_null() {
        return ptr::null_mut();
    }
    let hub_ptr = virtmcu_qom::qom::object_dynamic_cast(hub_obj, c"virtmcu-transport-hub".as_ptr());
    if hub_ptr.is_null() {
        return ptr::null_mut();
    }
    let s = &*(hub_ptr as *mut VirtmcuTransportHub);
    if s.rust_state.is_null() {
        return ptr::null_mut();
    }
    let state = &*(s.rust_state);
    if let Some(t) = &state._transport {
        &**t as *const alloc::sync::Arc<dyn virtmcu_api::DataTransport> as *mut core::ffi::c_void
    } else {
        core::ptr::null_mut()
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

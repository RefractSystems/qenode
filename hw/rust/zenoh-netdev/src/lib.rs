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
use std::ffi::{CStr, CString};
use std::ptr;
use virtmcu_qom::error::Error;
use virtmcu_qom::net::{
    qemu_new_net_client, virtmcu_zenoh_netdev_hook, NetClientInfo, NetClientState, Netdev,
    NET_CLIENT_DRIVER_ZENOH,
};
use virtmcu_qom::qdev::{DeviceClass, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::{declare_device_type, device_class, error_setg};
use zenoh::pubsub::Subscriber;
use zenoh::Session;
use zenoh::Wait;

#[repr(C)]
pub struct ZenohNetdevQEMU {
    pub parent_obj: SysBusDevice,
    pub nc: NetClientState,
    pub rust_state: *mut ZenohNetdevState,
}

pub struct ZenohNetdevState {
    session: Session,
    nc: *mut NetClientState,
    node_id: u32,
    topic: String,
    subscriber: Option<Subscriber<()>>,
}

unsafe extern "C" fn zenoh_netdev_receive(
    nc: *mut NetClientState,
    buf: *const u8,
    size: usize,
) -> isize {
    // Find ZenohNetdevQEMU from nc using offset_of
    let s = &mut *((nc as *mut u8).sub(core::mem::offset_of!(ZenohNetdevQEMU, nc))
        as *mut ZenohNetdevQEMU);
    if s.rust_state.is_null() {
        return 0;
    }
    zenoh_netdev_receive_internal(&*s.rust_state, buf, size)
}

unsafe extern "C" fn zenoh_netdev_can_receive(_nc: *mut NetClientState) -> bool {
    true
}

unsafe extern "C" fn zenoh_netdev_cleanup(nc: *mut NetClientState) {
    let s = &mut *((nc as *mut u8).sub(core::mem::offset_of!(ZenohNetdevQEMU, nc))
        as *mut ZenohNetdevQEMU);
    if !s.rust_state.is_null() {
        drop(Box::from_raw(s.rust_state));
        s.rust_state = ptr::null_mut();
    }
}

static NET_ZENOH_INFO: NetClientInfo = NetClientInfo {
    type_id: NET_CLIENT_DRIVER_ZENOH,
    size: std::mem::size_of::<ZenohNetdevQEMU>(),
    receive: Some(zenoh_netdev_receive),
    receive_raw: ptr::null_mut(),
    receive_iov: ptr::null_mut(),
    cleanup: Some(zenoh_netdev_cleanup),
    can_receive: Some(zenoh_netdev_can_receive),
    _opaque: [0; 208 - 56],
};

unsafe extern "C" fn zenoh_netdev_hook(
    netdev: *const Netdev,
    name: *const c_char,
    peer: *mut NetClientState,
    errp: *mut *mut Error,
) -> c_int {
    let opts = &(*netdev).u.zenoh;

    let nc = qemu_new_net_client(&NET_ZENOH_INFO, peer, c"zenoh".as_ptr(), name);
    let s = &mut *((nc as *mut u8).sub(core::mem::offset_of!(ZenohNetdevQEMU, nc))
        as *mut ZenohNetdevQEMU);

    let node_id = if opts.node.is_null() {
        0
    } else {
        CStr::from_ptr(opts.node)
            .to_string_lossy()
            .parse::<u32>()
            .unwrap_or(0)
    };

    let router = if opts.router.is_null() {
        ptr::null()
    } else {
        opts.router as *const c_char
    };

    let topic = if opts.topic.is_null() {
        "sim/net".to_string()
    } else {
        CStr::from_ptr(opts.topic).to_string_lossy().into_owned()
    };

    s.rust_state = zenoh_netdev_init_internal(nc, node_id, router, topic);
    if s.rust_state.is_null() {
        error_setg!(
            errp,
            c"zenoh-netdev: failed to initialize Rust backend".as_ptr()
        );
        return -1;
    }

    0
}

unsafe extern "C" fn zenoh_netdev_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).user_creatable = true;
        virtmcu_zenoh_netdev_hook = Some(zenoh_netdev_hook);
    }
}

static ZENOH_NETDEV_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-netdev".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: std::mem::size_of::<ZenohNetdevQEMU>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: 0,
    class_init: Some(zenoh_netdev_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(zenoh_netdev_type_init, ZENOH_NETDEV_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn zenoh_netdev_init_internal(
    nc: *mut NetClientState,
    node_id: u32,
    router: *const c_char,
    topic: String,
) -> *mut ZenohNetdevState {
    let session = unsafe {
        match virtmcu_zenoh::open_session(router) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    let full_topic = format!("{}/{}", topic, node_id);
    let nc_ptr = nc as usize;

    let subscriber = session
        .declare_subscriber(&full_topic)
        .callback(move |sample| {
            let nc = nc_ptr as *mut NetClientState;
            let data = sample.payload().to_bytes();
            unsafe {
                virtmcu_qom::net::qemu_send_packet(nc, data.as_ptr(), data.len());
            }
        })
        .wait()
        .ok();

    Box::into_raw(Box::new(ZenohNetdevState {
        session,
        nc,
        node_id,
        topic,
        subscriber,
    }))
}

fn zenoh_netdev_receive_internal(state: &ZenohNetdevState, buf: *const u8, size: usize) -> isize {
    let topic = format!("{}/{}", state.topic, state.node_id);
    let data = unsafe { std::slice::from_raw_parts(buf, size) };
    let _ = state.session.put(topic, data.to_vec()).wait();
    size as isize
}

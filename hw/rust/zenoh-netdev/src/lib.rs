#![allow(clippy::missing_safety_doc, clippy::collapsible_match, dead_code, unused_imports, clippy::len_zero)]
extern crate libc;

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::ptr;
use std::sync::Arc;
use zenoh::{Config, Session, Wait};
use zenoh::pubsub::Subscriber;
use crossbeam_channel::{bounded, Sender, Receiver};

pub struct ZenohNetdevBackend {
    session: Session,
    _subscriber: Subscriber<()>,
    node_id: u32,
    _topic: String,
    _nc: *mut c_void,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_init(
    node_id: u32,
    router: *const c_char,
    topic: *const c_char,
    nc: *mut c_void,
) -> *mut ZenohNetdevBackend {
    let mut config = Config::default();
    if !router.is_null() {
        if let Ok(r_str) = CStr::from_ptr(router).to_str() {
            if !r_str.is_empty() {
                let json = format!("[\"{}\"]", r_str);
                let _ = config.insert_json5("connect/endpoints", &json);
                let _ = config.insert_json5("scouting/multicast/enabled", "false");
            }
        }
    }

    let session = match zenoh::open(config).wait() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    let sub_topic = if !topic.is_null() {
        CStr::from_ptr(topic).to_str().unwrap_or("").to_string()
    } else {
        format!("sim/eth/frame/*/{node_id}")
    };

    let nc_usize = nc as usize;
    let subscriber = session.declare_subscriber(&sub_topic)
        .callback(move |sample| {
            let nc_ptr = nc_usize as *mut c_void;
            let data = sample.payload().to_bytes();
            virtmcu_bql_lock();
            qemu_receive_packet(nc_ptr, data.as_ptr(), data.len() as i32);
            virtmcu_bql_unlock();
        })
        .wait()
        .unwrap();

    Box::into_raw(Box::new(ZenohNetdevBackend {
        session,
        _subscriber: subscriber,
        node_id,
        _topic: sub_topic,
        _nc: nc,
    }))
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_send(backend: *mut ZenohNetdevBackend, buf: *const u8, size: usize) {
    let b = &*backend;
    let topic = format!("sim/eth/frame/{}/broadcast", b.node_id);
    let data = std::slice::from_raw_parts(buf, size);
    let _ = b.session.put(topic, data).wait();
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_netdev_free(backend: *mut ZenohNetdevBackend) {
    if !backend.is_null() {
        let _ = Box::from_raw(backend);
    }
}

extern "C" {
    fn virtmcu_bql_lock();
    fn virtmcu_bql_unlock();
    fn qemu_receive_packet(nc: *mut c_void, buf: *const u8, size: i32) -> isize;
}

#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]
extern crate libc;

use byteorder::{ByteOrder, LittleEndian};
use core::ffi::{c_char, c_void};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr;

use zenoh::pubsub::Publisher;
use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;

use virtmcu_qom::timer::*;

pub struct ZenohActuatorState {
    #[allow(dead_code)]
    session: Session,
    node_id: u32,
    topic_prefix: String,
    publishers: HashMap<u32, Publisher<'static>>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_actuator_init_rust(
    node_id: u32,
    router: *const c_char,
    topic_prefix: *const c_char,
) -> *mut ZenohActuatorState {
    let prefix = if !topic_prefix.is_null() {
        CStr::from_ptr(topic_prefix).to_str().unwrap().to_owned()
    } else {
        "firmware/control".to_owned()
    };

    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[zenoh-actuator] node={}: FAILED to open Zenoh session: {}",
                node_id, e
            );
            return ptr::null_mut();
        }
    };

    Box::into_raw(Box::new(ZenohActuatorState {
        session,
        node_id,
        topic_prefix: prefix,
        publishers: HashMap::new(),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_actuator_publish_rust(
    state: *mut ZenohActuatorState,
    actuator_id: u32,
    data_size: u32,
    data: *const f64,
) {
    let s = &mut *state;

    if !s.publishers.contains_key(&actuator_id) {
        let topic = format!("{}/{}/{}", s.topic_prefix, s.node_id, actuator_id);
        let pub_ = s.session.declare_publisher(topic).wait().unwrap();
        s.publishers.insert(actuator_id, pub_);
    }
    let publisher = s.publishers.get(&actuator_id).unwrap();

    let vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) as u64;
    let mut msg = Vec::with_capacity(8 + (data_size as usize) * 8);

    let mut vtime_bytes = [0u8; 8];
    LittleEndian::write_u64(&mut vtime_bytes, vtime);
    msg.extend_from_slice(&vtime_bytes);

    let slice = std::slice::from_raw_parts(data, data_size as usize);
    for &val in slice {
        let mut d_bytes = [0u8; 8];
        LittleEndian::write_f64(&mut d_bytes, val);
        msg.extend_from_slice(&d_bytes);
    }

    let _ = publisher.put(msg).wait();
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_actuator_cleanup_rust(state: *mut ZenohActuatorState) {
    if state.is_null() {
        return;
    }
    let _ = Box::from_raw(state);
}

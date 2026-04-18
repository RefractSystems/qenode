#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]
extern crate libc;

use core::ffi::c_char;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr;
use std::sync::{Arc, Mutex};

use zenoh::pubsub::Publisher;
use zenoh::pubsub::Subscriber;
use zenoh::Config;
use zenoh::Session;
use zenoh::Wait;

use virtmcu_qom::irq::*;
use virtmcu_qom::sync::*;

struct ZenohButton {
    #[allow(dead_code)]
    id: u32,
    state: bool,
    irq: SafeIrq,
    #[allow(dead_code)]
    subscriber: Subscriber<()>,
}

struct LedEvent {
    led_id: u32,
    state: bool,
}

pub struct ZenohUiState {
    session: Session,
    node_id: u32,
    buttons: Arc<Mutex<HashMap<u32, ZenohButton>>>,
    sender: Sender<Option<LedEvent>>,
    #[allow(dead_code)]
    publish_thread: Option<std::thread::JoinHandle<()>>,
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_ui_init_rust(
    node_id: u32,
    router: *const c_char,
) -> *mut ZenohUiState {
    let session = match virtmcu_zenoh::open_session(router) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[zenoh-ui] node={}: FAILED to open Zenoh session: {}",
                node_id, e
            );
            return ptr::null_mut();
        }
    };

    let (tx, rx) = bounded(64);
    let sess_clone = session.clone();
    let node_id_clone = node_id;
    let thread = std::thread::spawn(move || {
        ui_worker(rx, sess_clone, node_id_clone);
    });

    let state = Box::new(ZenohUiState {
        session,
        node_id,
        buttons: Arc::new(Mutex::new(HashMap::new())),
        sender: tx,
        publish_thread: Some(thread),
    });

    Box::into_raw(state)
}

fn ui_worker(rx: Receiver<Option<LedEvent>>, session: Session, node_id: u32) {
    let mut publishers: HashMap<u32, Publisher<'static>> = HashMap::new();
    while let Ok(Some(ev)) = rx.recv() {
        let pub_ = publishers.entry(ev.led_id).or_insert_with(|| {
            let topic = format!("sim/ui/{}/led/{}", node_id, ev.led_id);
            session.declare_publisher(topic).wait().unwrap()
        });
        let val: u8 = if ev.state { 1 } else { 0 };
        let _ = pub_.put(vec![val]).wait();
    }
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_ui_set_led_rust(state: *mut ZenohUiState, led_id: u32, on: bool) {
    let s = &*state;
    let _ = s.sender.try_send(Some(LedEvent { led_id, state: on }));
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_ui_get_button_rust(state: *mut ZenohUiState, btn_id: u32) -> bool {
    let s = &*state;
    let btns = s.buttons.lock().unwrap();
    btns.get(&btn_id).map(|b| b.state).unwrap_or(false)
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_ui_ensure_button_rust(
    state: *mut ZenohUiState,
    btn_id: u32,
    irq: qemu_irq,
) {
    let s = &mut *state;
    {
        let mut btns = s.buttons.lock().unwrap();
        if let Some(btn) = btns.get_mut(&btn_id) {
            btn.irq = SafeIrq(irq);
            return;
        }
    }

    let topic = format!("sim/ui/{}/button/{}", s.node_id, btn_id);
    let btns_clone = Arc::clone(&s.buttons);
    let btn_id_clone = btn_id;

    let subscriber = s
        .session
        .declare_subscriber(topic)
        .callback(move |sample| {
            let payload = sample.payload();
            if payload.len() < 1 {
                return;
            }
            let val = payload.to_bytes()[0] != 0;

            let mut btns = btns_clone.lock().unwrap();
            if let Some(btn) = btns.get_mut(&btn_id_clone) {
                if btn.state != val {
                    btn.state = val;
                    if !btn.irq.0.is_null() {
                        unsafe {
                            virtmcu_bql_lock();
                            qemu_set_irq(btn.irq.0, if val { 1 } else { 0 });
                            virtmcu_bql_unlock();
                        }
                    }
                }
            }
        })
        .wait()
        .unwrap();

    let mut btns = s.buttons.lock().unwrap();
    btns.insert(
        btn_id,
        ZenohButton {
            id: btn_id,
            state: false,
            irq: SafeIrq(irq),
            subscriber,
        },
    );
}

#[no_mangle]
pub unsafe extern "C" fn zenoh_ui_cleanup_rust(state: *mut ZenohUiState) {
    if state.is_null() {
        return;
    }
    let mut s = Box::from_raw(state);
    let _ = s.sender.send(None);
    if let Some(t) = s.publish_thread.take() {
        let _ = t.join();
    }
}

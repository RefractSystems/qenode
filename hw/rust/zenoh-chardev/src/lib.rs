use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::Cursor;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    virtmcu_timer_del, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};
use virtmcu_qom::{declare_device_type, vlog};
use zenoh::pubsub::Subscriber;
use zenoh::{Session, Wait};

#[repr(C)]
#[derive(Copy, Clone)]
struct ChardevZenohWrapper {
    data: *mut ChardevZenohOptions,
}

#[repr(C)]
union ChardevBackendUnion {
    zenoh: ChardevZenohWrapper,
    data: *mut c_void,
}

#[repr(C)]
struct ChardevBackend_Fields {
    type_: c_int,
    u: ChardevBackendUnion,
}

#[repr(C)]
pub struct ChardevZenohOptions {
    pub common: [u8; 8], // Placeholder for ChardevCommon
    _pad: [u8; 8],       // To match C layout if node is at offset 16
    pub node: *mut c_char,
    pub router: *mut c_char,
    pub topic: *mut c_char,
}

#[repr(C)]
pub struct ChardevZenoh {
    pub parent_obj: Chardev,
    pub rust_state: *mut ZenohChardevState,
}

pub struct ZenohChardevState {
    pub session: Session,
    pub topic: String,
    pub subscriber: Option<Subscriber<()>>,
    pub chr: *mut Chardev,
    pub rx_timer: *mut QemuTimer,
    pub timer_ptr: Arc<AtomicUsize>,
    pub rx_receiver: Receiver<Vec<u8>>,
    pub tx_sender: Sender<Vec<u8>>,
}

extern "C" {
    pub fn qemu_opt_get(opts: *mut c_void, name: *const c_char) -> *const c_char;
    pub fn g_strdup(s: *const c_char) -> *mut c_char;
    pub fn g_malloc0(size: usize) -> *mut c_void;
    pub fn g_free(p: *mut c_void);
    pub fn qemu_chr_parse_common(opts: *mut c_void, base: *mut c_void);
    pub fn get_chardev_backend_kind_zenoh() -> c_int;
    pub fn virtmcu_error_setg(errp: *mut *mut virtmcu_qom::error::Error, fmt: *const c_char);
    pub fn qemu_chr_be_write(s: *mut Chardev, buf: *const u8, len: usize);
}

unsafe extern "C" fn zenoh_chr_write(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int {
    let s = &mut *(chr as *mut ChardevZenoh);
    if s.rust_state.is_null() {
        return 0;
    }
    let state = &*s.rust_state;
    let data = std::slice::from_raw_parts(buf, len as usize);

    let _ = state.tx_sender.send(data.to_vec());
    len
}

unsafe extern "C" fn zenoh_chr_parse(
    opts: *mut c_void,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) {
    let node = qemu_opt_get(opts, c"node".as_ptr());

    if node.is_null() {
        let msg = c"chardev: zenoh: 'node' is required".as_ptr();
        virtmcu_error_setg(errp as *mut *mut _, msg);
        return;
    }

    let router = qemu_opt_get(opts, c"router".as_ptr());
    let topic = qemu_opt_get(opts, c"topic".as_ptr());

    let zenoh_opts =
        g_malloc0(std::mem::size_of::<ChardevZenohOptions>()) as *mut ChardevZenohOptions;
    (*zenoh_opts).node = g_strdup(node);
    if !router.is_null() {
        (*zenoh_opts).router = g_strdup(router);
    }
    if !topic.is_null() {
        (*zenoh_opts).topic = g_strdup(topic);
    }

    let b = &mut *(backend as *mut ChardevBackend_Fields);
    b.type_ = get_chardev_backend_kind_zenoh();
    b.u.zenoh = ChardevZenohWrapper { data: zenoh_opts };

    qemu_chr_parse_common(opts, zenoh_opts as *mut c_void);
}

extern "C" fn zenoh_chr_rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut ZenohChardevState) };
    while let Ok(payload) = state.rx_receiver.try_recv() {
        unsafe {
            qemu_chr_be_write(state.chr, payload.as_ptr(), payload.len());
        }
    }
}

fn send_packet(session: &Session, topic: &str, data: &[u8]) {
    // Header: [vtime(8) | len(4)]
    let vtime =
        unsafe { virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL) };
    let mut payload = Vec::with_capacity(12 + data.len());
    let _ = payload.write_u64::<LittleEndian>(vtime as u64);
    let _ = payload.write_u32::<LittleEndian>(data.len() as u32);
    payload.extend_from_slice(data);

    let _ = session.put(topic, payload).wait();
}

unsafe extern "C" fn zenoh_chr_open(
    chr: *mut Chardev,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) -> bool {
    vlog!("[zenoh-chardev] zenoh_chr_open called\n");
    let s = &mut *(chr as *mut ChardevZenoh);
    let b = &*(backend as *mut ChardevBackend_Fields);
    let wrapper = b.u.zenoh;
    let opts = wrapper.data;

    let node = CStr::from_ptr((*opts).node).to_string_lossy().into_owned();
    let router_ptr =
        if (*opts).router.is_null() { ptr::null() } else { (*opts).router.cast_const() };

    match virtmcu_zenoh::open_session(router_ptr) {
        Ok(session) => {
            let base_topic = if (*opts).topic.is_null() {
                "virtmcu/uart".to_string()
            } else {
                CStr::from_ptr((*opts).topic).to_string_lossy().into_owned()
            };

            let rx_topic = format!("{base_topic}/{node}/rx");
            let tx_topic = format!("{base_topic}/{node}/tx");

            let (tx, rx) = unbounded();
            let timer_ptr = Arc::new(AtomicUsize::new(0));
            let timer_ptr_clone = Arc::clone(&timer_ptr);

            let (tx_out, rx_out) = unbounded();

            let mut state = Box::new(ZenohChardevState {
                session,
                topic: tx_topic,
                subscriber: None,
                chr,
                rx_timer: ptr::null_mut(),
                timer_ptr,
                rx_receiver: rx,
                tx_sender: tx_out,
            });

            let sess_clone = state.session.clone();
            let topic_clone = state.topic.clone();
            std::thread::spawn(move || {
                let mut buffer = Vec::with_capacity(8192);
                let mut last_send = std::time::Instant::now();

                loop {
                    match rx_out.recv_timeout(std::time::Duration::from_millis(10)) {
                        Ok(data) => {
                            buffer.extend_from_slice(&data);
                            if buffer.len() >= 4096 || last_send.elapsed().as_millis() >= 20 {
                                send_packet(&sess_clone, &topic_clone, &buffer);
                                buffer.clear();
                                last_send = std::time::Instant::now();
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            if !buffer.is_empty() {
                                send_packet(&sess_clone, &topic_clone, &buffer);
                                buffer.clear();
                                last_send = std::time::Instant::now();
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
            });

            let sub = state
                .session
                .declare_subscriber(rx_topic)
                .callback(move |sample| {
                    let tp = timer_ptr_clone.load(AtomicOrdering::Acquire);
                    if tp == 0 {
                        return;
                    }
                    let rx_timer = tp as *mut QemuTimer;

                    let data = sample.payload().to_bytes();
                    let (payload, mut vtime) = if data.len() < 12 {
                        (data.to_vec(), 0)
                    } else {
                        let mut cursor = Cursor::new(&data);
                        let vt = cursor.read_u64::<LittleEndian>().unwrap_or(0);
                        let sz = cursor.read_u32::<LittleEndian>().unwrap_or(0);
                        let p = &data[12..];
                        let actual_len = std::cmp::min(sz as usize, p.len());
                        (p[..actual_len].to_vec(), vt)
                    };

                    if vtime == 0 {
                        vtime = unsafe {
                            virtmcu_qom::timer::qemu_clock_get_ns(
                                virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                            )
                        } as u64;
                    }

                    if tx.send(payload).is_ok() {
                        unsafe {
                            virtmcu_timer_mod(rx_timer, vtime as i64);
                        }
                    }
                })
                .wait();

            match sub {
                Ok(subscriber) => {
                    state.subscriber = Some(subscriber);
                    let state_ptr = &raw mut *state;
                    let rx_timer = unsafe {
                        virtmcu_timer_new_ns(
                            QEMU_CLOCK_VIRTUAL,
                            zenoh_chr_rx_timer_cb,
                            state_ptr as *mut c_void,
                        )
                    };
                    state.rx_timer = rx_timer;
                    state.timer_ptr.store(rx_timer as usize, AtomicOrdering::Release);

                    s.rust_state = Box::into_raw(state);
                    vlog!("[zenoh-chardev] zenoh_chr_open success\n");
                    true
                }
                Err(e) => {
                    let msg = format!("chardev: zenoh: failed to declare subscriber: {e}");
                    if let Ok(c_msg) = CString::new(msg) {
                        virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr());
                    }
                    false
                }
            }
        }
        Err(e) => {
            let msg = format!("chardev: zenoh: failed to open session: {e}");
            if let Ok(c_msg) = CString::new(msg) {
                virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr());
            }
            false
        }
    }
}

unsafe extern "C" fn zenoh_chr_finalize(obj: *mut Object) {
    vlog!("[zenoh-chardev] zenoh_chr_finalize called\n");
    let s = &mut *(obj as *mut ChardevZenoh);
    if !s.rust_state.is_null() {
        let mut state = Box::from_raw(s.rust_state);
        state.timer_ptr.store(0, AtomicOrdering::Release);
        if let Some(sub) = state.subscriber.take() {
            let _ = sub.undeclare().wait();
        }
        if !state.rx_timer.is_null() {
            virtmcu_timer_del(state.rx_timer);
            virtmcu_timer_free(state.rx_timer);
        }
        s.rust_state = ptr::null_mut();
    }
}

unsafe extern "C" fn char_zenoh_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    vlog!("[zenoh-chardev] char_zenoh_class_init called\n");
    let cc = &mut *(klass as *mut ChardevClass);
    cc.chr_parse = Some(zenoh_chr_parse);
    cc.chr_open = Some(zenoh_chr_open);
    cc.chr_write = Some(zenoh_chr_write);
}

static CHAR_ZENOH_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"chardev-zenoh".as_ptr(),
    parent: c"chardev".as_ptr(),
    instance_size: std::mem::size_of::<ChardevZenoh>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_chr_finalize),
    abstract_: false,
    class_size: std::mem::size_of::<ChardevClass>(),
    class_init: Some(char_zenoh_class_init),
    class_base_init: None,
    class_data: ptr::null_mut(),
    interfaces: ptr::null_mut(),
};

declare_device_type!(virtmcu_chardev_zenoh_init, CHAR_ZENOH_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chardev_zenoh_layout() {
        assert!(core::mem::offset_of!(ChardevZenohOptions, node) == 16);
        assert!(core::mem::size_of::<ChardevZenohOptions>() == 40);
        assert!(core::mem::size_of::<Chardev>() == 160);
    }
}

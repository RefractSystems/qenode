use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::Cursor;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use virtmcu_qom::sync::BqlGuarded;

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    virtmcu_timer_del, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
};
use virtmcu_qom::{declare_device_type, vlog};
use virtmcu_zenoh::SafeSubscriber;
use zenoh::{Session, Wait};

pub struct OrderedPacket {
    pub vtime: u64,
    pub data: Vec<u8>,
}

impl PartialEq for OrderedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime
    }
}
impl Eq for OrderedPacket {}
impl PartialOrd for OrderedPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        other.vtime.cmp(&self.vtime)
    }
}

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
    pub node: String,
    pub subscriber: Option<SafeSubscriber>,
    pub chr: *mut Chardev,
    pub rx_timer: *mut QemuTimer,
    pub kick_timer: *mut QemuTimer,
    pub timer_ptr: Arc<AtomicUsize>,
    pub rx_receiver: Receiver<OrderedPacket>,
    // All state accessed exclusively under BQL; see BqlGuarded docs.
    pub local_heap: BqlGuarded<BinaryHeap<OrderedPacket>>,
    pub backlog: BqlGuarded<VecDeque<u8>>,
    pub earliest_vtime: Arc<AtomicU64>,
    pub running: Arc<AtomicBool>,
    pub tx_sender: Option<Sender<Vec<u8>>>,
    pub tx_thread: Option<std::thread::JoinHandle<()>>,
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
    pub fn qemu_chr_be_can_write(s: *mut Chardev) -> c_int;
}

unsafe extern "C" fn zenoh_chr_write(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int {
    let s = &mut *(chr as *mut ChardevZenoh);
    if s.rust_state.is_null() {
        return 0;
    }
    let state = &*s.rust_state;
    let data = std::slice::from_raw_parts(buf, len as usize);

    if let Some(sender) = &state.tx_sender {
        let _ = sender.send(data.to_vec());
    }
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

fn drain_backlog(state: &ZenohChardevState) -> bool {
    let mut backlog = state.backlog.get_mut();
    if backlog.is_empty() {
        return false;
    }

    let can_write = unsafe { qemu_chr_be_can_write(state.chr) };
    if can_write <= 0 {
        return true;
    }

    let to_write = std::cmp::min(can_write as usize, backlog.len());
    let data: Vec<u8> = backlog.drain(..to_write).collect();
    unsafe {
        qemu_chr_be_write(state.chr, data.as_ptr(), data.len());
    }
    !backlog.is_empty()
}

extern "C" fn zenoh_chr_kick_timer_cb(opaque: *mut c_void) {
    zenoh_chr_rx_timer_cb(opaque);
}

extern "C" fn zenoh_chr_rx_timer_cb(opaque: *mut c_void) {
    let state = unsafe { &mut *(opaque as *mut ZenohChardevState) };
    let now =
        unsafe { virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL) }
            as u64;

    // First, try to drain any existing backlog. If it's still stalled, don't push more.
    if drain_backlog(state) {
        return;
    }

    let mut heap = state.local_heap.get_mut();

    // Drain receiver into heap
    while let Ok(packet) = state.rx_receiver.try_recv() {
        heap.push(packet);
    }

    // Process packets <= now
    let mut next_vtime = u64::MAX;
    while let Some(packet) = heap.peek() {
        if packet.vtime <= now {
            let can_write = unsafe { qemu_chr_be_can_write(state.chr) };
            if can_write <= 0 {
                // Cannot write anything, try again in 1ms (virtual time)
                next_vtime = now + 1_000_000;
                break;
            }

            if let Some(p) = heap.pop() {
                let to_write = std::cmp::min(can_write as usize, p.data.len());
                unsafe {
                    qemu_chr_be_write(state.chr, p.data.as_ptr(), to_write);
                }

                if to_write < p.data.len() {
                    // Buffer leftovers
                    let mut backlog = state.backlog.get_mut();
                    backlog.extend(&p.data[to_write..]);
                    // Try again in 1ms (virtual time)
                    next_vtime = now + 1_000_000;
                    break;
                }
            }
        } else {
            next_vtime = packet.vtime;
            break;
        }
    }

    if next_vtime == u64::MAX {
        state.earliest_vtime.store(u64::MAX, AtomicOrdering::Release);
    } else {
        state.earliest_vtime.store(next_vtime, AtomicOrdering::Release);
        unsafe {
            virtmcu_timer_mod(state.rx_timer, next_vtime as i64);
        }
    }
}

unsafe extern "C" fn zenoh_chr_accept_input(chr: *mut Chardev) {
    let s = &mut *(chr as *mut ChardevZenoh);
    if s.rust_state.is_null() {
        return;
    }
    let state = &*s.rust_state;
    let stalled = drain_backlog(state);

    if !stalled {
        virtmcu_timer_mod(state.rx_timer, 0); // 0 means ASAP (now)
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

    if let Err(e) = session.put(topic, payload).wait() {
        vlog!("[zenoh-chardev] Warning: Failed to send Zenoh packet: {}\n", e);
    }
}

fn start_tx_thread(
    session: zenoh::Session,
    tx_topic: String,
    rx_out: Receiver<Vec<u8>>,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = Vec::with_capacity(8192);
        let mut last_send = std::time::Instant::now();

        loop {
            if !running.load(AtomicOrdering::Acquire) && rx_out.is_empty() {
                break;
            }
            match rx_out.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(data) => {
                    buffer.extend_from_slice(&data);
                    if buffer.len() >= 4096 || last_send.elapsed().as_millis() >= 20 {
                        send_packet(&session, &tx_topic, &buffer);
                        buffer.clear();
                        last_send = std::time::Instant::now();
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if !buffer.is_empty() {
                        send_packet(&session, &tx_topic, &buffer);
                        buffer.clear();
                        last_send = std::time::Instant::now();
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    })
}

fn create_subscriber(
    session: &zenoh::Session,
    rx_topic: &str,
    kick_timer_ptr: Arc<AtomicUsize>,
    tx: Sender<OrderedPacket>,
    earliest_vtime: Arc<AtomicU64>,
) -> Result<SafeSubscriber, zenoh::Error> {
    SafeSubscriber::new(session, rx_topic, move |sample| {
        let tp = kick_timer_ptr.load(AtomicOrdering::Acquire);
        if tp == 0 {
            return;
        }
        let kick_timer = tp as *mut QemuTimer;

        let data = sample.payload().to_bytes();
        if data.len() < 12 {
            vlog!(
                "[zenoh-chardev] Warning: Dropping malformed packet (too short: {} bytes)\n",
                data.len()
            );
            return;
        }

        let mut cursor = Cursor::new(&data);
        let vtime = cursor.read_u64::<LittleEndian>().unwrap_or(0);
        let sz = cursor.read_u32::<LittleEndian>().unwrap_or(0);
        let p = &data[12..];
        let actual_len = std::cmp::min(sz as usize, p.len());
        let payload = p[..actual_len].to_vec();

        let adjusted_vtime = if vtime == 0 {
            unsafe {
                virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL) as u64
            }
        } else {
            vtime
        };

        if tx.send(OrderedPacket { vtime: adjusted_vtime, data: payload }).is_ok() {
            let current_earliest = earliest_vtime.load(AtomicOrdering::Acquire);
            if adjusted_vtime < current_earliest {
                earliest_vtime.store(adjusted_vtime, AtomicOrdering::Release);
                unsafe {
                    // Kick the main loop via a real-time timer (safe without BQL)
                    virtmcu_timer_mod(kick_timer, 0);
                }
            }
        } else {
            vlog!("[zenoh-chardev] Warning: RX channel full, dropping packet\n");
        }
    })
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

            // Bounded channel provides hardware backpressure
            let (tx, rx) = bounded(65536);
            let timer_ptr = Arc::new(AtomicUsize::new(0));
            let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

            let (tx_out, rx_out): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = bounded(65536);

            let running = Arc::new(AtomicBool::new(true));
            let tx_thread =
                start_tx_thread(session.clone(), tx_topic.clone(), rx_out, Arc::clone(&running));

            let mut state = Box::new(ZenohChardevState {
                session: session.clone(),
                topic: tx_topic,
                node,
                subscriber: None,
                chr,
                rx_timer: ptr::null_mut(),
                kick_timer: ptr::null_mut(),
                timer_ptr: Arc::clone(&timer_ptr),
                rx_receiver: rx,
                local_heap: BqlGuarded::new(BinaryHeap::new()),
                backlog: BqlGuarded::new(VecDeque::new()),
                earliest_vtime: Arc::clone(&earliest_vtime),
                running,
                tx_sender: Some(tx_out),
                tx_thread: Some(tx_thread),
            });

            let sub =
                create_subscriber(&session, &rx_topic, Arc::clone(&timer_ptr), tx, earliest_vtime);

            match sub {
                Ok(subscriber) => {
                    state.subscriber = Some(subscriber);
                    let state_ptr = &raw mut *state;

                    state.rx_timer = unsafe {
                        virtmcu_timer_new_ns(
                            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
                            zenoh_chr_rx_timer_cb,
                            state_ptr as *mut c_void,
                        )
                    };

                    state.kick_timer = unsafe {
                        virtmcu_timer_new_ns(
                            virtmcu_qom::timer::QEMU_CLOCK_REALTIME,
                            zenoh_chr_kick_timer_cb,
                            state_ptr as *mut c_void,
                        )
                    };

                    state.timer_ptr.store(state.kick_timer as usize, AtomicOrdering::Release);

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
        unsafe {
            let mut state = Box::from_raw(s.rust_state);
            state.running.store(false, AtomicOrdering::Release);
            state.timer_ptr.store(0, AtomicOrdering::Release);

            // Dropping the SafeSubscriber automatically undeclares and waits
            state.subscriber.take();

            if !state.rx_timer.is_null() {
                virtmcu_timer_del(state.rx_timer);
                virtmcu_timer_free(state.rx_timer);
            }
            if !state.kick_timer.is_null() {
                virtmcu_timer_del(state.kick_timer);
                virtmcu_timer_free(state.kick_timer);
            }

            // Drop the sender to signal the background thread to exit cleanly
            drop(state.tx_sender.take());
            if let Some(handle) = state.tx_thread.take() {
                let _ = handle.join();
            }

            s.rust_state = ptr::null_mut();
        }
    }
}

unsafe extern "C" fn char_zenoh_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    vlog!("[zenoh-chardev] char_zenoh_class_init called\n");
    let cc = &mut *(klass as *mut ChardevClass);
    cc.chr_parse = Some(zenoh_chr_parse);
    cc.chr_open = Some(zenoh_chr_open);
    cc.chr_write = Some(zenoh_chr_write);
    cc.chr_accept_input = Some(zenoh_chr_accept_input);
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

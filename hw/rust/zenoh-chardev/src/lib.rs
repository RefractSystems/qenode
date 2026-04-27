extern crate alloc;

use alloc::collections::{BinaryHeap, VecDeque};
use alloc::ffi::CString;
use alloc::sync::Arc;
use core::cmp::Ordering;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use crossbeam_channel::{bounded, Receiver, Sender};
use virtmcu_qom::sync::BqlGuarded;

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    virtmcu_timer_del, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};
use virtmcu_qom::{declare_device_type, vlog};
use virtmcu_zenoh::SafeSubscriber;
use zenoh::{Session, Wait};

pub struct OrderedPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

impl PartialEq for OrderedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.vtime == other.vtime && self.sequence == other.sequence
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
        match other.vtime.cmp(&self.vtime) {
            Ordering::Equal => other.sequence.cmp(&self.sequence),
            ord => ord,
        }
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

pub struct TxPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

pub struct ZenohChardevState {
    pub session: Arc<Session>,
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
    pub tx_fifo: BqlGuarded<VecDeque<u8>>,
    pub tx_timer: *mut QemuTimer,
    pub baud_delay_ns: BqlGuarded<u64>,
    pub earliest_vtime: Arc<AtomicU64>,
    pub running: Arc<AtomicBool>,
    pub tx_sender: Option<Sender<TxPacket>>,
    pub tx_thread: Option<std::thread::JoinHandle<()>>,
    pub tx_sequence: AtomicU64,
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

/// # Safety
/// This function is called by QEMU. chr must be a valid pointer to a Chardev instance.
#[no_mangle]
pub unsafe extern "C" fn zenoh_chr_write(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int {
    // SAFETY: chr is assumed to be a valid pointer of ChardevZenoh type as per QOM convention.
    let s = unsafe { &mut *(chr as *mut ChardevZenoh) };
    if s.rust_state.is_null() {
        return 0;
    }
    // SAFETY: rust_state is non-null and owned by the Chardev instance.
    let state = unsafe { &*s.rust_state };
    // SAFETY: buf is a valid pointer provided by QEMU with length len.
    let data = unsafe { core::slice::from_raw_parts(buf, len as usize) };

    let mut fifo = state.tx_fifo.get_mut();
    let was_empty = fifo.is_empty();
    fifo.extend(data.iter().copied());

    if was_empty && !data.is_empty() {
        // SAFETY: Safe to query clock under BQL
        let now = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        let delay = *state.baud_delay_ns.get();
        // SAFETY: Valid timer
        unsafe {
            virtmcu_qom::timer::virtmcu_timer_mod(state.tx_timer, now + delay as i64);
        }
    }
    len
}

/// # Safety
/// This function is called by QEMU to parse chardev options.
#[no_mangle]
pub unsafe extern "C" fn zenoh_chr_parse(
    opts: *mut c_void,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) {
    // SAFETY: opts is a valid QemuOpts pointer.
    let node = unsafe { qemu_opt_get(opts, c"node".as_ptr()) };

    if node.is_null() {
        let msg = c"chardev: zenoh: 'node' is required".as_ptr();
        // SAFETY: errp is a valid error pointer.
        unsafe { virtmcu_error_setg(errp as *mut *mut _, msg) };
        return;
    }

    // SAFETY: opts is a valid QemuOpts pointer.
    let router = unsafe { qemu_opt_get(opts, c"router".as_ptr()) };
    // SAFETY: opts is a valid QemuOpts pointer.
    let topic = unsafe { qemu_opt_get(opts, c"topic".as_ptr()) };

    // SAFETY: All pointers are validated or strdup'd.
    let zenoh_opts = unsafe {
        let p = g_malloc0(core::mem::size_of::<ChardevZenohOptions>()) as *mut ChardevZenohOptions;
        (*p).node = g_strdup(node);
        if !router.is_null() {
            (*p).router = g_strdup(router);
        }
        if !topic.is_null() {
            (*p).topic = g_strdup(topic);
        }
        p
    };

    // SAFETY: backend is a valid ChardevBackend pointer.
    let b = unsafe { &mut *(backend as *mut ChardevBackend_Fields) };
    // SAFETY: type kind is retrieved from a safe C shim.
    b.type_ = unsafe { get_chardev_backend_kind_zenoh() };
    b.u.zenoh = ChardevZenohWrapper { data: zenoh_opts };

    // SAFETY: zenoh_opts is a valid pointer to ChardevZenohOptions.
    unsafe { qemu_chr_parse_common(opts, zenoh_opts as *mut c_void) };
}

extern "C" fn zenoh_chr_tx_timer_cb(opaque: *mut core::ffi::c_void) {
    // SAFETY: Provided by QEMU
    let s = unsafe { &mut *(opaque as *mut ChardevZenoh) };
    // SAFETY: s is a valid pointer
    let rust_state = s.rust_state;
    if rust_state.is_null() {
        return;
    }
    // SAFETY: Valid pointer
    let state = unsafe { &*rust_state };

    let mut fifo = state.tx_fifo.get_mut();
    if let Some(byte) = fifo.pop_front() {
        // SAFETY: Safe to query clock under BQL
        let vtime = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        let sequence = state.tx_sequence.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        if let Some(sender) = &state.tx_sender {
            let _ = sender.send(TxPacket { vtime: vtime as u64, sequence, data: vec![byte] });
        }
    }

    if !fifo.is_empty() {
        // SAFETY: Safe to query clock under BQL
        let now = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        let delay = *state.baud_delay_ns.get();
        // SAFETY: Valid timer
        unsafe {
            virtmcu_qom::timer::virtmcu_timer_mod(state.tx_timer, now + delay as i64);
        }
    }
}

#[repr(C)]
pub struct QEMUSerialSetParams {
    pub speed: core::ffi::c_int,
    pub parity: core::ffi::c_int,
    pub data_bits: core::ffi::c_int,
    pub stop_bits: core::ffi::c_int,
}
const CHR_IOCTL_SERIAL_SET_PARAMS: core::ffi::c_int = 1;

/// # Safety
/// Called by QEMU to handle Chardev ioctls.
#[no_mangle]
pub unsafe extern "C" fn zenoh_chr_ioctl(
    chr: *mut Chardev,
    cmd: core::ffi::c_int,
    arg: *mut c_void,
) -> core::ffi::c_int {
    // SAFETY: Provided by QEMU
    let s = unsafe { &mut *(chr as *mut ChardevZenoh) };
    if s.rust_state.is_null() {
        return -1; // ENOTSUP
    }
    // SAFETY: Valid pointer
    let state = unsafe { &*s.rust_state };

    if cmd == CHR_IOCTL_SERIAL_SET_PARAMS {
        if !arg.is_null() {
            // SAFETY: Provided by QEMU
            let ssp = unsafe { &*(arg as *mut QEMUSerialSetParams) };
            if ssp.speed > 0 {
                let delay = (1_000_000_000_u64 / (ssp.speed as u64)) * 10;
                *state.baud_delay_ns.get_mut() = delay;
                vlog!(
                    "[zenoh-chardev] Configured baud rate: {} bps (delay: {} ns)\n",
                    ssp.speed,
                    delay
                );
            }
        }
        return 0;
    }
    -1
}

// SAFETY: Internal helper to split initialization
unsafe fn init_zenoh_chardev_timers(state: &mut ZenohChardevState, s: *mut ChardevZenoh) {
    let state_ptr = &raw mut *state;
    // SAFETY: Creating timers is safe
    state.rx_timer = unsafe {
        virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, zenoh_chr_rx_timer_cb, state_ptr as *mut c_void)
    };
    // SAFETY: Creating timers is safe
    state.kick_timer = unsafe {
        virtmcu_timer_new_ns(
            virtmcu_qom::timer::QEMU_CLOCK_REALTIME,
            zenoh_chr_kick_timer_cb,
            state_ptr as *mut c_void,
        )
    };
    // SAFETY: Creating timers is safe
    state.tx_timer = unsafe {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            zenoh_chr_tx_timer_cb,
            core::ptr::from_mut(&mut *s) as *mut core::ffi::c_void,
        )
    };
    state.timer_ptr.store(state.kick_timer as usize, core::sync::atomic::Ordering::Release);
}

fn drain_backlog(state: &ZenohChardevState) -> bool {
    // SAFETY: Accessing QEMU clock is safe within BQL context.
    let now =
        unsafe { virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL) }
            as u64;

    // 1. First, drain any existing byte-level backlog
    {
        let mut backlog = state.backlog.get_mut();
        if !backlog.is_empty() {
            // SAFETY: chr is a valid pointer to Chardev.
            let can_write = unsafe { qemu_chr_be_can_write(state.chr) };
            if can_write <= 0 {
                return true; // Still stalled
            }
            let to_write = core::cmp::min(can_write as usize, backlog.len());
            let data: Vec<u8> = backlog.drain(..to_write).collect();
            // SAFETY: qemu_chr_be_write expects valid buffer and length.
            unsafe {
                qemu_chr_be_write(state.chr, data.as_ptr(), data.len());
            }
            if !backlog.is_empty() {
                return true; // Still stalled
            }
        }
    }

    // 2. Next, process ONE packet from the heap that is ready (vtime <= now)
    let mut heap = state.local_heap.get_mut();
    // Move any pending packets from receiver to heap first
    while let Ok(mut packet) = state.rx_receiver.try_recv() {
        if packet.vtime == 0 {
            packet.vtime = now;
        }
        heap.push(packet);
    }

    if let Some(packet) = heap.peek() {
        if packet.vtime <= now {
            // SAFETY: chr is a valid pointer to Chardev.
            let can_write = unsafe { qemu_chr_be_can_write(state.chr) };
            if can_write <= 0 {
                return true; // Stalled
            }

            if let Some(p) = heap.pop() {
                let to_write = core::cmp::min(can_write as usize, p.data.len());
                // SAFETY: qemu_chr_be_write expects valid buffer and length.
                unsafe {
                    qemu_chr_be_write(state.chr, p.data.as_ptr(), to_write);
                }

                if to_write < p.data.len() {
                    // Buffer leftovers in byte backlog and wait for next accept_input
                    let mut backlog = state.backlog.get_mut();
                    backlog.extend(&p.data[to_write..]);
                    return true; // Stalled
                }
            }
        }
    }

    // Return true if there's STILL more ready work to do, false if we're done for now.
    // This allows the caller to re-trigger if needed.
    heap.peek().is_some_and(|p| p.vtime <= now)
}

extern "C" fn zenoh_chr_rx_timer_cb(opaque: *mut c_void) {
    // SAFETY: opaque is a valid pointer to ZenohChardevState.
    let state = unsafe { &mut *(opaque as *mut ZenohChardevState) };

    // Try to drain everything ready. Process in a loop but with a safety limit
    // to avoid hogging the BQL for too long in a single timer callback.
    let mut count = 0;
    let mut stalled = false;
    while count < 10 {
        stalled = drain_backlog(state);
        if stalled {
            // Stalled, stop for now.
            break;
        }
        count += 1;
    }

    // Schedule next wakeup
    let mut next_vtime = u64::MAX;

    if stalled {
        // If we're stalled, we must wait for either accept_input OR a tiny bit of virtual time
        // to avoid tight polling if the guest doesn't call accept_input.
        // SAFETY: Accessing QEMU clock is safe within BQL context.
        let now = unsafe {
            virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL)
        } as u64;
        next_vtime = now + 1_000_000; // 1ms virtual time
    } else {
        // Not stalled, check if there are future packets in the heap
        let heap = state.local_heap.get();
        // SAFETY: Accessing QEMU clock is safe within BQL context.
        let now = unsafe {
            virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL)
        } as u64;
        if let Some(packet) = heap.peek() {
            if packet.vtime <= now {
                // More ready work! Re-schedule timer for NOW (0)
                next_vtime = 0;
            } else {
                next_vtime = packet.vtime;
            }
        }
    }

    if next_vtime == u64::MAX {
        state.earliest_vtime.store(u64::MAX, AtomicOrdering::Release);
    } else {
        state.earliest_vtime.store(next_vtime, AtomicOrdering::Release);
        // SAFETY: rx_timer is a valid QemuTimer pointer.
        unsafe {
            virtmcu_timer_mod(state.rx_timer, next_vtime as i64);
        }
    }
}

extern "C" fn zenoh_chr_kick_timer_cb(opaque: *mut c_void) {
    zenoh_chr_rx_timer_cb(opaque);
}

/// # Safety
/// This function is called by QEMU when the backend can accept more data.
#[no_mangle]
pub unsafe extern "C" fn zenoh_chr_accept_input(chr: *mut Chardev) {
    // SAFETY: chr is a valid pointer to ChardevZenoh.
    let s = unsafe { &mut *(chr as *mut ChardevZenoh) };
    if s.rust_state.is_null() {
        return;
    }
    // SAFETY: rust_state is non-null and owned by the Chardev instance.
    let state = unsafe { &*s.rust_state };

    // Guest is ready for more data. Try to drain immediately.
    let stalled = drain_backlog(state);

    if !stalled {
        // Successfully pushed data, check if we need to schedule the timer for future packets
        let heap = state.local_heap.get();
        if let Some(packet) = heap.peek() {
            state.earliest_vtime.store(packet.vtime, AtomicOrdering::Release);
            // SAFETY: rx_timer is a valid QemuTimer pointer.
            unsafe { virtmcu_timer_mod(state.rx_timer, packet.vtime as i64) };
        } else {
            state.earliest_vtime.store(u64::MAX, AtomicOrdering::Release);
        }
    }
}

fn send_packet(session: &Session, topic: &str, packet: TxPacket) {
    use virtmcu_api::ZenohFrameHeader;
    let header = ZenohFrameHeader {
        delivery_vtime_ns: packet.vtime,
        sequence_number: packet.sequence,
        size: packet.data.len() as u32,
    };
    let mut payload = Vec::with_capacity(20 + packet.data.len());
    payload.extend_from_slice(&header.pack());
    payload.extend_from_slice(&packet.data);

    if let Err(e) = session.put(topic, payload).wait() {
        vlog!("[zenoh-chardev] Warning: Failed to send Zenoh packet: {}\n", e);
    }
}

fn start_tx_thread(
    session: Arc<Session>,
    tx_topic: String,

    rx_out: Receiver<TxPacket>,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = Vec::with_capacity(8192);
        let mut first_vtime = 0;
        let mut first_seq = 0;
        let mut last_send = std::time::Instant::now();

        loop {
            if !running.load(AtomicOrdering::Acquire) && rx_out.is_empty() {
                break;
            }
            match rx_out.recv_timeout(core::time::Duration::from_millis(10)) {
                Ok(packet) => {
                    if buffer.is_empty() {
                        first_vtime = packet.vtime;
                        first_seq = packet.sequence;
                    }
                    buffer.extend_from_slice(&packet.data);
                    if buffer.len() >= 4096 || last_send.elapsed().as_millis() >= 20 {
                        send_packet(
                            &session,
                            &tx_topic,
                            TxPacket {
                                vtime: first_vtime,
                                sequence: first_seq,
                                data: buffer.clone(),
                            },
                        );
                        buffer.clear();
                        last_send = std::time::Instant::now();
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if !buffer.is_empty() {
                        send_packet(
                            &session,
                            &tx_topic,
                            TxPacket {
                                vtime: first_vtime,
                                sequence: first_seq,
                                data: buffer.clone(),
                            },
                        );
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
) -> Result<SafeSubscriber, zenoh::Error> {
    use virtmcu_api::ZenohFrameHeader;
    SafeSubscriber::new(session, rx_topic, move |sample| {
        let tp = kick_timer_ptr.load(AtomicOrdering::Acquire);
        if tp == 0 {
            return;
        }
        let kick_timer = tp as *mut QemuTimer;

        let data = sample.payload().to_bytes();
        if data.len() < 20 {
            vlog!(
                "[zenoh-chardev] Warning: Dropping malformed packet (too short: {} bytes)\n",
                data.len()
            );
            return;
        }

        let header = match ZenohFrameHeader::unpack_slice(&data[..20]) {
            Some(h) => h,
            None => return,
        };

        let p = &data[20..];
        let actual_len = core::cmp::min(header.size as usize, p.len());
        let payload = p[..actual_len].to_vec();

        // If vtime=0, it means "deliver now". We queue it as vtime=0 and let the
        // timer callback assign the correct virtual time once it holds the BQL.
        if tx
            .send(OrderedPacket {
                vtime: header.delivery_vtime_ns,
                sequence: header.sequence_number,
                data: payload,
            })
            .is_ok()
        {
            // SAFETY: kick_timer is a valid QemuTimer pointer.
            unsafe {
                // Kick the main loop via a real-time timer (safe without BQL)
                virtmcu_timer_mod(kick_timer, 0);
            }
        } else {
            vlog!("[zenoh-chardev] Warning: RX channel full, dropping packet\n");
        }
    })
}

/// # Safety
/// This function is called by QEMU when opening the chardev.
#[no_mangle]
pub unsafe extern "C" fn zenoh_chr_open(
    chr: *mut Chardev,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) -> bool {
    vlog!("[zenoh-chardev] zenoh_chr_open called\n");
    // SAFETY: chr is a valid pointer to ChardevZenoh.
    let s = unsafe { &mut *(chr as *mut ChardevZenoh) };
    // SAFETY: backend is a valid ChardevBackend pointer.
    let b = unsafe { &*(backend as *mut ChardevBackend_Fields) };
    let wrapper = b.u.zenoh;
    let opts = wrapper.data;

    // SAFETY: opts->node is a valid pointer.
    let node = unsafe { CStr::from_ptr((*opts).node).to_string_lossy().into_owned() };
    // SAFETY: opts->router can be null.
    let router_ptr = unsafe {
        if (*opts).router.is_null() {
            ptr::null()
        } else {
            (*opts).router.cast_const()
        }
    };

    // SAFETY: router_ptr is safe as get_or_init_session handles null and valid pointers.
    // Safety: router validity is guaranteed by the caller.
    match unsafe { virtmcu_zenoh::get_or_init_session(router_ptr) } {
        Ok(session) => {
            // SAFETY: opts->topic can be null.
            let base_topic = unsafe {
                if (*opts).topic.is_null() {
                    "virtmcu/uart".to_string()
                } else {
                    CStr::from_ptr((*opts).topic).to_string_lossy().into_owned()
                }
            };

            let rx_topic = format!("{base_topic}/{node}/rx");
            let tx_topic = format!("{base_topic}/{node}/tx");

            // Bounded channel provides hardware backpressure
            let (tx, rx) = bounded(65536);
            let timer_ptr = Arc::new(AtomicUsize::new(0));
            let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

            let (tx_out, rx_out): (Sender<TxPacket>, Receiver<TxPacket>) = bounded(65536);

            let running = Arc::new(AtomicBool::new(true));
            let tx_thread = start_tx_thread(
                Arc::clone(&session),
                tx_topic.clone(),
                rx_out,
                Arc::clone(&running),
            );

            let mut state = Box::new(ZenohChardevState {
                session: Arc::clone(&session),
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
                tx_fifo: BqlGuarded::new(VecDeque::new()),
                tx_timer: ptr::null_mut(),
                baud_delay_ns: BqlGuarded::new(86800), // Default 115200 bps
                earliest_vtime: Arc::clone(&earliest_vtime),
                running,
                tx_sender: Some(tx_out),
                tx_thread: Some(tx_thread),
                tx_sequence: AtomicU64::new(0),
            });

            let sub = create_subscriber(&session, &rx_topic, Arc::clone(&timer_ptr), tx);

            match sub {
                Ok(subscriber) => {
                    state.subscriber = Some(subscriber);
                    // SAFETY: Safe to initialize timers
                    unsafe { init_zenoh_chardev_timers(&mut state, s) };

                    s.rust_state = Box::into_raw(state);
                    vlog!("[zenoh-chardev] zenoh_chr_open success\n");
                    true
                }
                Err(e) => {
                    let msg = format!("chardev: zenoh: failed to declare subscriber: {e}");
                    if let Ok(c_msg) = CString::new(msg) {
                        // SAFETY: errp is a valid error pointer.
                        unsafe { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
                    }
                    false
                }
            }
        }
        Err(e) => {
            let msg = format!("chardev: zenoh: failed to open session: {e}");
            if let Ok(c_msg) = CString::new(msg) {
                // SAFETY: errp is a valid error pointer.
                unsafe { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
            }
            false
        }
    }
}

/// # Safety
/// This function is called by QEMU when finalizing the chardev.
#[no_mangle]
pub unsafe extern "C" fn zenoh_chr_finalize(obj: *mut Object) {
    vlog!("[zenoh-chardev] zenoh_chr_finalize called\n");
    // SAFETY: obj is a valid pointer to ChardevZenoh.
    let s = unsafe { &mut *(obj as *mut ChardevZenoh) };
    if !s.rust_state.is_null() {
        // SAFETY: rust_state was allocated via Box::into_raw and is non-null.
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

/// # Safety
/// This function is called by QEMU to initialize the chardev class.
#[no_mangle]
pub unsafe extern "C" fn char_zenoh_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    vlog!("[zenoh-chardev] char_zenoh_class_init called\n");
    // SAFETY: klass is a valid pointer to ChardevClass.
    let cc = unsafe { &mut *(klass as *mut ChardevClass) };
    cc.chr_parse = Some(zenoh_chr_parse);
    cc.chr_open = Some(zenoh_chr_open);
    cc.chr_write = Some(zenoh_chr_write);
    cc.chr_accept_input = Some(zenoh_chr_accept_input);
    cc.chr_ioctl = Some(zenoh_chr_ioctl);
}

static CHAR_ZENOH_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"chardev-zenoh".as_ptr(),
    parent: c"chardev".as_ptr(),
    instance_size: core::mem::size_of::<ChardevZenoh>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_chr_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<ChardevClass>(),
    class_init: Some(char_zenoh_class_init),
    class_base_init: None,
    class_data: ptr::null_mut(),
    interfaces: ptr::null_mut(),
};

declare_device_type!(VIRTMCU_CHARDEV_ZENOH_TYPE_INIT, CHAR_ZENOH_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chardev_zenoh_layout() {
        assert!(core::mem::offset_of!(ChardevZenohOptions, node) == 16);
        assert!(core::mem::size_of::<ChardevZenohOptions>() == 56);
        assert!(core::mem::size_of::<Chardev>() == 160);
    }
}

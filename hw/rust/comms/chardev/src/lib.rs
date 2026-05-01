extern crate alloc;

use alloc::collections::{BinaryHeap, VecDeque};
use alloc::ffi::CString;
use alloc::sync::Arc;
use core::cmp::Ordering;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use core::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use std::sync::{Condvar, Mutex};
use virtmcu_qom::sync::{Bql, BqlGuarded};

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    virtmcu_timer_del, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};

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
struct ChardevVirtmcuWrapper {
    data: *mut ChardevVirtmcuOptions,
}

#[repr(C)]
union ChardevBackendUnion {
    virtmcu: ChardevVirtmcuWrapper,
    _data: *mut c_void,
}

#[repr(C)]
struct ChardevBackend_Fields {
    _type: c_int,
    u: ChardevBackendUnion,
}

#[repr(C)]
pub struct ChardevVirtmcuOptions {
    /* Members inherited from ChardevCommon: */
    pub logfile: *mut c_char,
    pub has_logappend: bool,
    pub logappend: bool,
    pub has_logtimestamp: bool,
    pub logtimestamp: bool,
    _pad_common: [u8; 4],
    /* Own members: */
    pub node: *mut c_char,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub topic: *mut c_char,
    pub has_max_backlog: bool,
    _pad_own: [u8; 7],
    pub max_backlog: u64,
}

#[repr(C)]
pub struct ChardevVirtmcu {
    pub parent_obj: Chardev,
    pub rust_state: *mut VirtmcuChardevState,
}

pub struct TxPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

pub struct VirtmcuChardevState {
    pub shared: Arc<SharedState>,
    pub chr: *mut Chardev,
    pub rx_timer: *mut QemuTimer,
    pub rx_baud_timer: *mut QemuTimer,
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
    pub tx_thread: Option<std::thread::JoinHandle<()>>,
    pub tx_sequence: AtomicU64,
    pub max_backlog: u64,
    pub backlog_size_atomic: Arc<AtomicU64>,
    pub dropped_frames_atomic: Arc<AtomicU64>,
}

pub struct InnerState {
    pub running: bool,
    pub active_vcpu_count: usize,
}

pub struct SharedState {
    pub transport: Box<dyn virtmcu_api::DataTransport>,
    pub topic: String,
    pub node: String,
    pub subscription: Mutex<Option<virtmcu_qom::sync::SafeSubscription>>, // MUTEX_EXCEPTION: shared with QEMU callbacks
    pub tx_sender: Sender<TxPacket>,
    pub drain_cond: Condvar,
    pub state: Mutex<InnerState>, // MUTEX_EXCEPTION: shared for lifecycle
}

extern "C" {
    pub fn qemu_opt_get(opts: *mut c_void, name: *const c_char) -> *const c_char;
    pub fn qemu_opt_get_size(opts: *mut c_void, name: *const c_char, defval: u64) -> u64;
    pub fn g_strdup(s: *const c_char) -> *mut c_char;
    pub fn g_malloc0(size: usize) -> *mut c_void;
    pub fn g_free(p: *mut c_void);
    pub fn qemu_chr_parse_common(opts: *mut c_void, base: *mut c_void);
    pub fn virtmcu_error_setg(errp: *mut *mut virtmcu_qom::error::Error, fmt: *const c_char);
    pub fn qemu_chr_be_write(s: *mut Chardev, buf: *const u8, len: usize);
    pub fn qemu_chr_be_can_write(s: *mut Chardev) -> c_int;
}

struct VcpuCountGuard<'a>(&'a SharedState);
impl Drop for VcpuCountGuard<'_> {
    fn drop(&mut self) {
        let mut lock = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        lock.active_vcpu_count = lock.active_vcpu_count.saturating_sub(1);
        if lock.active_vcpu_count == 0 {
            self.0.drain_cond.notify_all();
        }
    }
}

/// # Safety
/// This function is called by QEMU. chr must be a valid pointer to a Chardev instance.
#[no_mangle]
pub unsafe extern "C" fn virtmcu_chr_write(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int {
    // SAFETY: chr is assumed to be a valid pointer of ChardevVirtmcu type as per QOM convention.
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) };
    if s.rust_state.is_null() {
        return 0;
    }
    // SAFETY: rust_state is non-null and owned by the Chardev instance.
    let state = unsafe { &*s.rust_state };

    {
        let mut lock = state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !lock.running {
            return 0;
        }
        lock.active_vcpu_count += 1;
    }
    let _guard = VcpuCountGuard(&state.shared);

    // SAFETY: buf is a valid pointer provided by QEMU with length len.
    let data = unsafe { core::slice::from_raw_parts(buf, len as usize) };

    let mut fifo = state.tx_fifo.get_mut();
    let was_empty = fifo.is_empty();
    if fifo.len() + data.len() <= 65536 {
        fifo.extend(data.iter().copied());
    } else {
        // Drop data if FIFO is full to prevent unbounded memory growth.
        virtmcu_qom::sim_info!("TX FIFO overflow, dropping {} bytes", data.len());
    }

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
pub unsafe extern "C" fn virtmcu_chr_parse(
    opts: *mut c_void,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) {
    // SAFETY: opts is a valid QemuOpts pointer.
    let node = unsafe { qemu_opt_get(opts, c"node".as_ptr()) };

    if node.is_null() {
        let msg = c"chardev: virtmcu: 'node' is required".as_ptr();
        // SAFETY: errp is a valid error pointer.
        unsafe { virtmcu_error_setg(errp as *mut *mut _, msg) };
        return;
    }

    // SAFETY: opts is a valid QemuOpts pointer.
    let transport = unsafe { qemu_opt_get(opts, c"transport".as_ptr()) };
    let router = unsafe { qemu_opt_get(opts, c"router".as_ptr()) };
    let topic = unsafe { qemu_opt_get(opts, c"topic".as_ptr()) };
    let max_backlog_str = unsafe { qemu_opt_get(opts, c"max-backlog".as_ptr()) };

    // SAFETY: All pointers are validated or strdup'd.
    let virtmcu_opts = unsafe {
        let p =
            g_malloc0(core::mem::size_of::<ChardevVirtmcuOptions>()) as *mut ChardevVirtmcuOptions;
        // 1. Parse common chardev options (logfile, logappend, etc)
        qemu_chr_parse_common(opts, p as *mut c_void);

        // 2. Parse VirtMCU specific options
        (*p).node = g_strdup(node);
        if !transport.is_null() {
            (*p).transport = g_strdup(transport);
        }
        if !router.is_null() {
            (*p).router = g_strdup(router);
        }
        if !topic.is_null() {
            (*p).topic = g_strdup(topic);
        }

        if max_backlog_str.is_null() {
            (*p).has_max_backlog = false;
            (*p).max_backlog = 256;
        } else {
            (*p).has_max_backlog = true;
            (*p).max_backlog = qemu_opt_get_size(opts, c"max-backlog".as_ptr(), 256);
        }
        p
    };

    // SAFETY: backend is a valid ChardevBackend pointer.
    let b = unsafe { &mut *(backend as *mut ChardevBackend_Fields) };
    b.u.virtmcu = ChardevVirtmcuWrapper { data: virtmcu_opts };

    // SAFETY: virtmcu_opts is a valid pointer to ChardevVirtmcuOptions.
    unsafe { qemu_chr_parse_common(opts, virtmcu_opts as *mut c_void) };
}

extern "C" fn virtmcu_chr_tx_timer_cb(opaque: *mut core::ffi::c_void) {
    // SAFETY: Provided by QEMU
    let s = unsafe { &mut *(opaque as *mut ChardevVirtmcu) };
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
        match state.shared.tx_sender.try_send(TxPacket {
            vtime: vtime as u64,
            sequence,
            data: vec![byte],
        }) {
            Ok(_) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                virtmcu_qom::sim_info!("TX channel full, dropping packet");
            }
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
pub unsafe extern "C" fn virtmcu_chr_ioctl(
    chr: *mut Chardev,
    cmd: core::ffi::c_int,
    arg: *mut c_void,
) -> core::ffi::c_int {
    // SAFETY: Provided by QEMU
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) };
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
                virtmcu_qom::sim_info!("{} bps (delay: {} ns)", ssp.speed, delay);
            }
        }
        return 0;
    }
    -1
}

// SAFETY: Internal helper to split initialization
unsafe fn init_chardev_timers(state: &mut VirtmcuChardevState, s: *mut ChardevVirtmcu) {
    let state_ptr = core::ptr::from_mut::<VirtmcuChardevState>(&mut *state);
    // SAFETY: Creating timers is safe
    state.rx_timer = unsafe {
        virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, virtmcu_chr_rx_timer_cb, state_ptr as *mut c_void)
    };
    // SAFETY: Creating timers is safe
    state.kick_timer = unsafe {
        virtmcu_timer_new_ns(
            virtmcu_qom::timer::QEMU_CLOCK_REALTIME,
            virtmcu_chr_kick_timer_cb,
            state_ptr as *mut c_void,
        )
    };
    // SAFETY: Creating timers is safe
    state.tx_timer = unsafe {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            virtmcu_chr_tx_timer_cb,
            core::ptr::from_mut(&mut *s) as *mut core::ffi::c_void,
        )
    };
    // SAFETY: Creating timers is safe
    state.rx_baud_timer = unsafe {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            virtmcu_chr_rx_baud_timer_cb,
            state_ptr as *mut c_void,
        )
    };
    state.timer_ptr.store(state.kick_timer as usize, core::sync::atomic::Ordering::Release);
}

extern "C" fn virtmcu_chr_rx_baud_timer_cb(opaque: *mut core::ffi::c_void) {
    // SAFETY: Provided by QEMU
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) };

    let mut backlog = state.backlog.get_mut();
    if backlog.is_empty() {
        return;
    }

    // SAFETY: chr is a valid pointer.
    let can_write = unsafe { qemu_chr_be_can_write(state.chr) };
    if can_write > 0 {
        if let Some(byte) = backlog.pop_front() {
            let data = [byte];
            // SAFETY: qemu_chr_be_write expects valid buffer and length.
            unsafe {
                qemu_chr_be_write(state.chr, data.as_ptr(), 1);
            }
        }
    }

    if !backlog.is_empty() && can_write > 0 {
        // SAFETY: Safe to query clock under BQL
        let now = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        let delay = *state.baud_delay_ns.get();
        // SAFETY: Valid timer
        unsafe {
            virtmcu_qom::timer::virtmcu_timer_mod(state.rx_baud_timer, now + delay as i64);
        }
    }
}

fn drain_backlog(state: &mut VirtmcuChardevState) -> bool {
    // SAFETY: Safe to query clock under BQL
    let now = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;

    // 1. Next, process ONE packet from the heap that is ready (vtime <= now)
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
            if let Some(p) = heap.pop() {
                state.backlog_size_atomic.fetch_sub(p.data.len() as u64, AtomicOrdering::SeqCst);
                let mut backlog = state.backlog.get_mut();
                let was_empty = backlog.is_empty();
                backlog.extend(&p.data);

                if was_empty && !backlog.is_empty() {
                    // SAFETY: Safe to query clock under BQL.
                    let now_ns =
                        unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
                    let delay = *state.baud_delay_ns.get();
                    // SAFETY: Valid timer
                    unsafe {
                        virtmcu_qom::timer::virtmcu_timer_mod(
                            state.rx_baud_timer,
                            now_ns + delay as i64,
                        );
                    }
                }
            }
        }
    }

    // Return true if there's STILL more ready work to do, false if we're done for now.
    heap.peek().is_some_and(|p| p.vtime <= now)
}

extern "C" fn virtmcu_chr_rx_timer_cb(opaque: *mut c_void) {
    // SAFETY: opaque is a valid pointer to VirtmcuChardevState.
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) };

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

extern "C" fn virtmcu_chr_kick_timer_cb(opaque: *mut c_void) {
    virtmcu_chr_rx_timer_cb(opaque);
}

/// # Safety
/// This function is called by QEMU when the backend can accept more data.
#[no_mangle]
pub unsafe extern "C" fn virtmcu_chr_accept_input(chr: *mut Chardev) {
    // SAFETY: chr is a valid pointer to ChardevVirtmcu.
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) };
    if s.rust_state.is_null() {
        return;
    }
    // SAFETY: rust_state is non-null and owned by the Chardev instance.
    let state = unsafe { &mut *s.rust_state };

    // Guest is ready for more data. Try to drain immediately.
    let stalled = drain_backlog(state);

    if !state.backlog.get().is_empty() {
        // Resume pushing bytes into the guest
        let now = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        unsafe { virtmcu_qom::timer::virtmcu_timer_mod(state.rx_baud_timer, now) };
    }

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

fn send_packet(transport: &dyn virtmcu_api::DataTransport, topic: &str, packet: TxPacket) {
    use virtmcu_api::{FlatBufferStructExt, ZenohFrameHeader};
    let header = ZenohFrameHeader::new(packet.vtime, packet.sequence, packet.data.len() as u32);
    let mut payload = Vec::with_capacity(virtmcu_api::ZENOH_FRAME_HEADER_SIZE + packet.data.len());
    payload.extend_from_slice(header.pack());
    payload.extend_from_slice(&packet.data);

    if let Err(e) = transport.publish(topic, &payload) {
        virtmcu_qom::sim_err!("{}", e);
    }
}

fn start_tx_thread(
    shared: Arc<SharedState>,
    rx_out: Receiver<TxPacket>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = Vec::with_capacity(8192);
        let mut first_vtime = 0;
        let mut first_seq = 0;
        let mut last_send = std::time::Instant::now();

        loop {
            {
                let lock = shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if !lock.running && rx_out.is_empty() {
                    break;
                }
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
                            &*shared.transport,
                            &shared.topic,
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
                            &*shared.transport,
                            &shared.topic,
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

unsafe fn add_chardev_properties(chr: *mut Chardev, state: &VirtmcuChardevState) {
    virtmcu_qom::qom::object_property_add_uint64_ptr(
        chr as *mut _,
        c"max-backlog".as_ptr(),
        &state.max_backlog,
        virtmcu_qom::qom::OBJ_PROP_FLAG_READ,
    );
    virtmcu_qom::qom::object_property_add_uint64_ptr(
        chr as *mut _,
        c"dropped-frames".as_ptr(),
        state.dropped_frames_atomic.as_ptr(),
        virtmcu_qom::qom::OBJ_PROP_FLAG_READ,
    );
    virtmcu_qom::qom::object_property_add_uint64_ptr(
        chr as *mut _,
        c"backlog-size".as_ptr(),
        state.backlog_size_atomic.as_ptr(),
        virtmcu_qom::qom::OBJ_PROP_FLAG_READ,
    );
}

unsafe fn parse_chardev_options(
    opts: *mut ChardevVirtmcuOptions,
) -> (String, String, *const c_char, String, u64) {
    let node = CStr::from_ptr((*opts).node).to_string_lossy().into_owned();

    let transport = if (*opts).transport.is_null() {
        "zenoh".to_owned()
    } else {
        CStr::from_ptr((*opts).transport).to_string_lossy().into_owned()
    };

    let router_ptr =
        if (*opts).router.is_null() { ptr::null() } else { (*opts).router.cast_const() };

    let base_topic = if (*opts).topic.is_null() {
        "virtmcu/uart".to_owned()
    } else {
        CStr::from_ptr((*opts).topic).to_string_lossy().into_owned()
    };

    let max_backlog = if (*opts).has_max_backlog { (*opts).max_backlog } else { 256 };

    (node, transport, router_ptr, base_topic, max_backlog)
}

fn create_chardev_transport(
    transport_name: &str,
    node: &str,
    router_ptr: *const c_char,
    errp: *mut *mut c_void,
) -> Option<Box<dyn virtmcu_api::DataTransport>> {
    if transport_name == "unix" {
        let path = if router_ptr.is_null() {
            format!("/tmp/virtmcu-coord-{}.sock", { node })
        } else {
            unsafe { core::ffi::CStr::from_ptr(router_ptr).to_string_lossy().into_owned() }
        };
        match transport_unix::UnixDataTransport::new(&path) {
            Ok(t) => Some(Box::new(t)),
            Err(e) => {
                let msg = format!("chardev: virtmcu: failed to open unix socket {path}: {e}");
                if let Ok(c_msg) = CString::new(msg) {
                    unsafe { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
                }
                None
            }
        }
    } else {
        // Default to Zenoh
        match unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(session) => Some(Box::new(transport_zenoh::ZenohDataTransport::new(session))),
            Err(e) => {
                let msg = format!("chardev: virtmcu: failed to open zenoh session: {e}");
                if let Ok(c_msg) = CString::new(msg) {
                    unsafe { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
                }
                None
            }
        }
    }
}

fn create_chardev_sub_callback(
    timer_ptr_clone: Arc<AtomicUsize>,
    backlog_size_clone: Arc<AtomicU64>,
    dropped_frames_clone: Arc<AtomicU64>,
    max_backlog: u64,
    tx: Sender<OrderedPacket>,
) -> virtmcu_api::DataCallback {
    Box::new(move |data| {
        use virtmcu_api::{FlatBufferStructExt, ZenohFrameHeader};
        let tp = timer_ptr_clone.load(AtomicOrdering::Acquire);
        if tp == 0 {
            return;
        }
        let kick_timer = tp as *mut QemuTimer;

        if data.len() < virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
            return;
        }

        let header =
            match ZenohFrameHeader::unpack_slice(&data[..virtmcu_api::ZENOH_FRAME_HEADER_SIZE]) {
                Some(h) => h,
                None => return,
            };

        let p = &data[virtmcu_api::ZENOH_FRAME_HEADER_SIZE..];
        let actual_len = core::cmp::min(header.size() as usize, p.len());
        virtmcu_qom::sim_err!(
            "Zenoh rx: len={}, p.len={}, hdr.size={}, actual={}",
            data.len(),
            p.len(),
            header.size(),
            actual_len
        );
        let payload = p[..actual_len].to_vec();

        // Backlog Admission Control (bytes)
        if backlog_size_clone.load(AtomicOrdering::SeqCst) + payload.len() as u64 > max_backlog {
            dropped_frames_clone.fetch_add(1, AtomicOrdering::SeqCst);
            return;
        }

        let payload_len = payload.len() as u64;
        if tx
            .try_send(OrderedPacket {
                vtime: header.delivery_vtime_ns(),
                sequence: header.sequence_number(),
                data: payload,
            })
            .is_ok()
        {
            backlog_size_clone.fetch_add(payload_len, AtomicOrdering::SeqCst);
            // SAFETY: kick_timer is a valid QemuTimer pointer.
            unsafe {
                virtmcu_timer_mod(kick_timer, 0);
            }
        } else {
            dropped_frames_clone.fetch_add(1, AtomicOrdering::SeqCst);
        }
    })
}

/// # Safety
/// This function is called by QEMU when opening the chardev.
#[no_mangle]
pub unsafe extern "C" fn virtmcu_chr_open(
    chr: *mut Chardev,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) -> bool {
    virtmcu_qom::sim_info!("virtmcu_chr_open called");
    // SAFETY: chr is a valid pointer to ChardevVirtmcu.
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) };
    // SAFETY: backend is a valid ChardevBackend pointer.
    let b = unsafe { &*(backend as *mut ChardevBackend_Fields) };
    let wrapper = b.u.virtmcu;
    let opts = wrapper.data;

    let (node, transport_name, router_ptr, base_topic, max_backlog) = parse_chardev_options(opts);

    let transport = match create_chardev_transport(&transport_name, &node, router_ptr, errp) {
        Some(t) => t,
        None => return false,
    };

    let rx_topic = format!("{base_topic}/{node}/rx");
    let tx_topic = format!("{base_topic}/{node}/tx");

    let (tx, rx) = bounded(1024);
    let timer_ptr = Arc::new(AtomicUsize::new(0));
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

    let (tx_out, rx_out): (Sender<TxPacket>, Receiver<TxPacket>) = bounded(1024);
    let backlog_size_atomic = Arc::new(AtomicU64::new(0));
    let dropped_frames_atomic = Arc::new(AtomicU64::new(0));

    let shared = Arc::new(SharedState {
        transport,
        topic: tx_topic,
        node,
        subscription: Mutex::new(None),
        tx_sender: tx_out,
        drain_cond: Condvar::new(),
        state: Mutex::new(InnerState { running: true, active_vcpu_count: 0 }),
    });

    let tx_thread = start_tx_thread(Arc::clone(&shared), rx_out);

    let mut state = Box::new(VirtmcuChardevState {
        shared: Arc::clone(&shared),
        chr,
        rx_timer: ptr::null_mut(),
        rx_baud_timer: ptr::null_mut(),
        kick_timer: ptr::null_mut(),
        timer_ptr: Arc::clone(&timer_ptr),
        rx_receiver: rx,
        local_heap: BqlGuarded::new(BinaryHeap::new()),
        backlog: BqlGuarded::new(VecDeque::new()),
        tx_fifo: BqlGuarded::new(VecDeque::new()),
        tx_timer: ptr::null_mut(),
        baud_delay_ns: BqlGuarded::new(86800), // Default 115200 bps
        earliest_vtime: Arc::clone(&earliest_vtime),
        tx_thread: Some(tx_thread),
        tx_sequence: AtomicU64::new(0),
        max_backlog,
        backlog_size_atomic: Arc::clone(&backlog_size_atomic),
        dropped_frames_atomic: Arc::clone(&dropped_frames_atomic),
    });

    // Add QOM properties for observability
    // SAFETY: chr is a valid pointer to a Chardev instance.
    unsafe { add_chardev_properties(chr, &state) };

    let timer_ptr_clone = Arc::clone(&timer_ptr);
    let backlog_size_clone = Arc::clone(&backlog_size_atomic);
    let dropped_frames_clone = Arc::clone(&dropped_frames_atomic);

    let sub_callback = create_chardev_sub_callback(
        timer_ptr_clone,
        backlog_size_clone,
        dropped_frames_clone,
        max_backlog,
        tx,
    );

    let generation = Arc::new(AtomicU64::new(0)); // chardev doesn't use generations yet
    let sub = virtmcu_qom::sync::SafeSubscription::new(
        &*shared.transport,
        &rx_topic,
        generation,
        sub_callback,
    );

    match sub {
        Ok(subscription) => {
            {
                let mut lock =
                    shared.subscription.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                *lock = Some(subscription);
            }
            // SAFETY: Safe to initialize timers
            unsafe { init_chardev_timers(&mut state, s) };

            s.rust_state = Box::into_raw(state);
            virtmcu_qom::sim_info!("virtmcu_chr_open success");
            true
        }
        Err(e) => {
            let msg = format!("chardev: virtmcu: failed to subscribe: {e}");
            if let Ok(c_msg) = CString::new(msg) {
                unsafe { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
            }
            false
        }
    }
}

/// # Safety
/// This function is called by QEMU when finalizing the chardev.
#[no_mangle]
pub unsafe extern "C" fn virtmcu_chr_finalize(obj: *mut Object) {
    virtmcu_qom::sim_info!("virtmcu_chr_finalize called");
    // SAFETY: obj is a valid pointer to ChardevVirtmcu.
    let s = unsafe { &mut *(obj as *mut ChardevVirtmcu) };
    if !s.rust_state.is_null() {
        // SAFETY: rust_state was allocated via Box::into_raw and is non-null.
        unsafe {
            let mut state = Box::from_raw(s.rust_state);
            {
                let mut lock =
                    state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                lock.running = false;
            }
            state.timer_ptr.store(0, AtomicOrdering::Release);

            // Dropping the SafeSubscription automatically undeclares and waits
            {
                let mut lock = state
                    .shared
                    .subscription
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                lock.take();
            }

            if !state.rx_timer.is_null() {
                virtmcu_timer_del(state.rx_timer);
                virtmcu_timer_free(state.rx_timer);
            }
            if !state.kick_timer.is_null() {
                virtmcu_timer_del(state.kick_timer);
                virtmcu_timer_free(state.kick_timer);
            }
            if !state.tx_timer.is_null() {
                virtmcu_timer_del(state.tx_timer);
                virtmcu_timer_free(state.tx_timer);
            }
            if !state.rx_baud_timer.is_null() {
                virtmcu_timer_del(state.rx_baud_timer);
                virtmcu_timer_free(state.rx_baud_timer);
            }

            // Wait for all vCPU threads to drain (bounded: avoids permanent deadlock)
            let mut lock =
                state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            while lock.active_vcpu_count > 0 {
                let bql_unlock = Bql::temporary_unlock();
                let (new_lock, timed_out) = state
                    .shared
                    .drain_cond
                    .wait_timeout(lock, Duration::from_secs(30))
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                lock = new_lock;
                drop(bql_unlock);
                if timed_out.timed_out() {
                    virtmcu_qom::sim_info!(
                        "drain timeout after 30 s ({} vCPU threads still active); proceeding with teardown",
                        lock.active_vcpu_count
                    );
                    break;
                }
            }

            if let Some(handle) = state.tx_thread.take() {
                let bql_unlock = Bql::temporary_unlock();
                let _ = handle.join();
                drop(bql_unlock);
            }

            s.rust_state = ptr::null_mut();
        }
    }
}

/// # Safety
/// This function is called by QEMU to initialize the chardev class.
#[no_mangle]
pub unsafe extern "C" fn char_virtmcu_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    virtmcu_qom::sim_info!("char_virtmcu_class_init called");
    // SAFETY: klass is a valid pointer to ChardevClass.
    let cc = unsafe { &mut *(klass as *mut ChardevClass) };
    cc.chr_parse = Some(virtmcu_chr_parse);
    cc.chr_open = Some(virtmcu_chr_open);
    cc.chr_write = Some(virtmcu_chr_write);
    cc.chr_accept_input = Some(virtmcu_chr_accept_input);
    cc.chr_ioctl = Some(virtmcu_chr_ioctl);
}

static CHAR_VIRTMCU_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"chardev-virtmcu".as_ptr(),
    parent: c"chardev".as_ptr(),
    instance_size: core::mem::size_of::<ChardevVirtmcu>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(virtmcu_chr_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<ChardevClass>(),
    class_init: Some(char_virtmcu_class_init),
    class_base_init: None,
    class_data: ptr::null_mut(),
    interfaces: ptr::null_mut(),
};

declare_device_type!(VIRTMCU_CHARDEV_VIRTMCU_TYPE_INIT, CHAR_VIRTMCU_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chardev_virtmcu_layout() {
        assert!(core::mem::offset_of!(ChardevVirtmcuOptions, logfile) == 0);
        assert!(core::mem::offset_of!(ChardevVirtmcuOptions, node) == 16);
        assert!(core::mem::offset_of!(ChardevVirtmcuOptions, has_max_backlog) == 48);
        assert!(core::mem::offset_of!(ChardevVirtmcuOptions, max_backlog) == 56);
        assert!(core::mem::size_of::<ChardevVirtmcuOptions>() == 64);
        assert!(core::mem::size_of::<Chardev>() == 160);
    }
}

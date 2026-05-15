#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
use zenoh::Wait;
extern crate alloc;

use alloc::collections::VecDeque;
use alloc::ffi::CString;
use alloc::sync::Arc;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use virtmcu_qom::cosim::{CoSimBridge, CoSimContext, CoSimTransport};

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::timer::{
    virtmcu_timer_del, virtmcu_timer_free, virtmcu_timer_mod, virtmcu_timer_new_ns, QemuTimer,
    QEMU_CLOCK_VIRTUAL,
};

const MAX_FIFO_SIZE: usize = 65536;
const MAX_BACKLOG: u64 = 256;
const SEND_BUF_CAPACITY: usize = 8192;
const FLUSH_THRESHOLD: usize = 4096;
const FLUSH_INTERVAL_MS: u64 = 20;
const DEFAULT_BAUD_DELAY_NS: u64 = 86800;
const SERIAL_BITS_PER_CHAR: u64 = 10;
const RECV_TIMEOUT_MS: u64 = 10;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrderedPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

impl virtmcu_qom::sync::DeliveryPacket for OrderedPacket {
    fn delivery_vtime_ns(&self) -> u64 {
        self.vtime
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
    pub has_baud_rate_ns: bool,
    _pad_own: [u8; 6],
    pub max_backlog: u64,
    pub baud_rate_ns: u64,
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
    pub bridge: CoSimBridge<ChardevTransport>,
    pub tx_sender: Sender<TxPacket>,
    pub chr: *mut Chardev,
    pub rx_timer: *mut QemuTimer,
    pub rx_baud_timer: *mut QemuTimer,
    pub kick_timer: *mut QemuTimer,
    pub timer_ptr: Arc<AtomicUsize>,
    pub receiver: Option<virtmcu_qom::sync::DeterministicReceiver<OrderedPacket>>,
    // All state accessed securely; see Mutex docs.
    pub backlog: virtmcu_qom::sync::Mutex<VecDeque<u8>>, // virtmcu-allow: mutex reasoning="Backlog managed securely"
    pub tx_fifo: Arc<virtmcu_qom::sync::Mutex<VecDeque<u8>>>, // virtmcu-allow: mutex reasoning="TX FIFO managed securely"
    pub tx_timer: *mut QemuTimer,
    pub tx_timer_ptr: Arc<AtomicUsize>,
    pub baud_delay_ns: Arc<AtomicU64>,
    pub earliest_vtime: Arc<AtomicU64>,
    pub tx_sequence: AtomicU64,
    pub max_backlog: u64,
    pub backlog_size_atomic: Arc<AtomicU64>,
    pub dropped_frames_atomic: Arc<AtomicU64>,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

extern "C" {
    pub fn qemu_opt_get(opts: *mut c_void, name: *const c_char) -> *const c_char;
    pub fn qemu_opt_get_size(opts: *mut c_void, name: *const c_char, defval: u64) -> u64;
    pub fn qemu_opt_get_number(opts: *mut c_void, name: *const c_char, defval: u64) -> u64;
    pub fn g_strdup(s: *const c_char) -> *mut c_char;
    pub fn g_malloc0(size: usize) -> *mut c_void;
    pub fn g_free(p: *mut c_void);
    pub fn qemu_chr_parse_common(opts: *mut c_void, base: *mut c_void);
    pub fn virtmcu_error_setg(errp: *mut *mut virtmcu_qom::error::Error, fmt: *const c_char);
    pub fn qemu_chr_be_write(s: *mut Chardev, buf: *const u8, len: usize);
    pub fn qemu_chr_be_can_write(s: *mut Chardev) -> c_int;
}

fn decode_chardev(_opaque: *mut c_void, _topic: &str, data: &[u8]) -> Option<OrderedPacket> {
    use virtmcu_api::{FlatBufferStructExt, ZenohFrameHeader};
    if data.len() < virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
        return None;
    }
    let header =
        ZenohFrameHeader::unpack(data[..virtmcu_api::ZENOH_FRAME_HEADER_SIZE].try_into().ok()?)?;
    let p = &data[virtmcu_api::ZENOH_FRAME_HEADER_SIZE..];
    let actual_len = core::cmp::min(header.size() as usize, p.len());
    let payload = p[..actual_len].to_vec();

    Some(OrderedPacket {
        vtime: header.delivery_vtime_ns(),
        sequence: header.sequence_number(),
        data: payload,
    })
}

fn deliver_chardev(opaque: *mut c_void, packet: OrderedPacket) {
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) };
    let mut backlog = state.backlog.lock(); // virtmcu-allow: mutex reasoning="Backlog managed securely"

    if state.backlog_size_atomic.load(AtomicOrdering::SeqCst) + packet.data.len() as u64
        > state.max_backlog
    {
        state.dropped_frames_atomic.fetch_add(1, AtomicOrdering::SeqCst);
        return;
    }

    let was_empty = backlog.is_empty();
    let payload_len = packet.data.len() as u64;
    backlog.extend(&packet.data);
    state.backlog_size_atomic.fetch_add(payload_len, AtomicOrdering::SeqCst);

    if was_empty && !backlog.is_empty() {
        unsafe {
            let now = virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
            virtmcu_timer_mod(state.rx_baud_timer, now);
        }
    }

    unsafe {
        virtmcu_timer_mod(state.rx_timer, 0);
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

    // SAFETY: buf is a valid pointer provided by QEMU with length len.
    let data = unsafe { core::slice::from_raw_parts(buf, len as usize) };

    state.bridge.send_and_wait(data.to_vec(), 0);
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
    let baud_rate_ns_str = unsafe { qemu_opt_get(opts, c"baud-rate-ns".as_ptr()) };

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
            (*p).max_backlog = MAX_BACKLOG;
        } else {
            (*p).has_max_backlog = true;
            (*p).max_backlog = qemu_opt_get_size(opts, c"max-backlog".as_ptr(), MAX_BACKLOG);
        }

        if baud_rate_ns_str.is_null() {
            (*p).has_baud_rate_ns = false;
            (*p).baud_rate_ns = DEFAULT_BAUD_DELAY_NS; // Default 115200 bps
        } else {
            (*p).has_baud_rate_ns = true;
            (*p).baud_rate_ns =
                qemu_opt_get_number(opts, c"baud-rate-ns".as_ptr(), DEFAULT_BAUD_DELAY_NS);
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

    let mut fifo = state.tx_fifo.lock(); // virtmcu-allow: mutex reasoning="TX FIFO managed securely"
    if let Some(byte) = fifo.pop_front() {
        // SAFETY: Safe to query clock under BQL
        let vtime = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        let sequence = state.tx_sequence.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        match state.tx_sender.try_send(TxPacket { vtime: vtime as u64, sequence, data: vec![byte] })
        {
            Ok(_) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                virtmcu_qom::sim_info!("TX channel full, dropping packet");
            }
        }
    }

    if !fifo.is_empty() {
        // SAFETY: Safe to query clock under BQL
        let now = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        let delay = state.baud_delay_ns.load(AtomicOrdering::Relaxed);
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
                let delay = (1_000_000_000_u64 / (ssp.speed as u64)) * SERIAL_BITS_PER_CHAR;
                state.baud_delay_ns.store(delay, AtomicOrdering::Relaxed);
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
    state.tx_timer_ptr.store(state.tx_timer as usize, core::sync::atomic::Ordering::Release);
}

extern "C" fn virtmcu_chr_rx_baud_timer_cb(opaque: *mut core::ffi::c_void) {
    // SAFETY: Provided by QEMU
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) };

    let mut backlog = state.backlog.lock(); // virtmcu-allow: mutex reasoning="Backlog managed securely"
    if backlog.is_empty() {
        return;
    }

    // SAFETY: chr is a valid pointer.
    let can_write = unsafe { qemu_chr_be_can_write(state.chr) };
    if can_write > 0 {
        if let Some(byte) = backlog.pop_front() {
            let data = [byte];
            state.backlog_size_atomic.fetch_sub(1, AtomicOrdering::SeqCst);
            // SAFETY: qemu_chr_be_write expects valid buffer and length.
            unsafe {
                qemu_chr_be_write(state.chr, data.as_ptr(), 1);
            }
        }
    }

    if !backlog.is_empty() && can_write > 0 {
        // SAFETY: Safe to query clock under BQL
        let now = unsafe { virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) };
        let delay = state.baud_delay_ns.load(AtomicOrdering::Relaxed);
        // SAFETY: Valid timer
        unsafe {
            virtmcu_qom::timer::virtmcu_timer_mod(state.rx_baud_timer, now + delay as i64);
        }
    }
}

fn drain_backlog(_state: &mut VirtmcuChardevState) -> bool {
    false
}

extern "C" fn virtmcu_chr_rx_timer_cb(opaque: *mut c_void) {
    // SAFETY: opaque is a valid pointer to VirtmcuChardevState.
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) };
    drain_backlog(state);
}

extern "C" fn virtmcu_chr_kick_timer_cb(opaque: *mut c_void) {
    virtmcu_chr_rx_timer_cb(opaque);
}

#[no_mangle]
pub unsafe extern "C" fn virtmcu_chr_accept_input(chr: *mut Chardev) {
    // SAFETY: chr is a valid pointer to ChardevVirtmcu.
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) };
    if s.rust_state.is_null() {
        return;
    }
    // SAFETY: rust_state is non-null and owned by the Chardev instance.
    let state = unsafe { &mut *s.rust_state };

    if !state.backlog.lock().is_empty() {
        // virtmcu-allow: mutex reasoning="Backlog managed securely"
        // Resume pushing bytes into the guest
        unsafe {
            let now = virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL);
            virtmcu_qom::timer::virtmcu_timer_mod(state.rx_baud_timer, now);
        };
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
    virtmcu_qom::qom::object_property_add_uint64_ptr(
        chr as *mut _,
        c"baud-rate-ns".as_ptr(),
        state.baud_delay_ns.as_ptr(),
        virtmcu_qom::qom::OBJ_PROP_FLAG_READ,
    );
}

unsafe fn parse_chardev_options(
    opts: *mut ChardevVirtmcuOptions,
) -> (String, String, *const c_char, String, u64, u64) {
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

    let max_backlog = if (*opts).has_max_backlog { (*opts).max_backlog } else { MAX_BACKLOG };

    let baud_delay_ns = if (*opts).has_baud_rate_ns {
        (*opts).baud_rate_ns
    } else {
        DEFAULT_BAUD_DELAY_NS // Default 115200 bps
    };

    (node, transport, router_ptr, base_topic, max_backlog, baud_delay_ns)
}

fn create_chardev_transport(
    transport_name: &str,
    node: &str,
    router_ptr: *const c_char,
    errp: *mut *mut c_void,
) -> Option<Arc<dyn virtmcu_api::DataTransport>> {
    if transport_name == "unix" {
        let path = if router_ptr.is_null() {
            format!("/tmp/virtmcu-coord-{node}.sock") // virtmcu-allow: absolute_path reasoning="Legacy script"
        } else {
            unsafe { core::ffi::CStr::from_ptr(router_ptr).to_string_lossy().into_owned() }
        };
        match transport_unix::UdsDataTransport::new(&path) {
            Ok(t) => Some(Arc::new(t) as Arc<dyn virtmcu_api::DataTransport>),
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
            Ok(session) => Some(Arc::new(transport_zenoh::ZenohDataTransport::new(session))
                as Arc<dyn virtmcu_api::DataTransport>),
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

    let (node, transport_name, router_ptr, base_topic, max_backlog, baud_delay_ns) =
        parse_chardev_options(opts);

    let transport = match create_chardev_transport(&transport_name, &node, router_ptr, errp) {
        Some(t) => t,
        None => return false,
    };

    let rx_topic = format!("{base_topic}/{node}/rx");
    let tx_topic = format!("{base_topic}/{node}/tx");

    let timer_ptr = Arc::new(AtomicUsize::new(0));
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

    let (tx_out, rx_out): (Sender<TxPacket>, Receiver<TxPacket>) = bounded(1024);
    let backlog_size_atomic = Arc::new(AtomicU64::new(0));
    let dropped_frames_atomic = Arc::new(AtomicU64::new(0));
    let tx_fifo = Arc::new(virtmcu_qom::sync::Mutex::new(VecDeque::new()));
    let baud_delay_ns_arc = Arc::new(AtomicU64::new(baud_delay_ns));
    let tx_timer_ptr = Arc::new(AtomicUsize::new(0));

    let transport_impl = ChardevTransport {
        transport: Arc::clone(&transport),
        topic: tx_topic,
        rx_out,
        tx_fifo: Arc::clone(&tx_fifo),
        baud_delay_ns: Arc::clone(&baud_delay_ns_arc),
        tx_timer_ptr: Arc::clone(&tx_timer_ptr),
    };
    let bridge = CoSimBridge::new(transport_impl);

    let liveliness = if transport_name == "zenoh" {
        match unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(session) => {
                let hb_topic = format!("sim/chardev/liveliness/{node}");
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let mut state = Box::new(VirtmcuChardevState {
        _liveliness: liveliness,
        bridge,
        tx_sender: tx_out,
        chr,
        rx_timer: ptr::null_mut(),
        rx_baud_timer: ptr::null_mut(),
        kick_timer: ptr::null_mut(),
        timer_ptr: Arc::clone(&timer_ptr),
        receiver: None,
        backlog: virtmcu_qom::sync::Mutex::new(VecDeque::new()), // virtmcu-allow: mutex reasoning="Backlog managed securely"
        tx_fifo,
        tx_timer: ptr::null_mut(),
        tx_timer_ptr,
        baud_delay_ns: baud_delay_ns_arc,
        earliest_vtime: Arc::clone(&earliest_vtime),
        tx_sequence: AtomicU64::new(0),
        max_backlog,
        backlog_size_atomic: Arc::clone(&backlog_size_atomic),
        dropped_frames_atomic: Arc::clone(&dropped_frames_atomic),
    });

    // Add QOM properties for observability
    // SAFETY: chr is a valid pointer to a Chardev instance.
    unsafe { add_chardev_properties(chr, &state) };

    let generation = Arc::new(AtomicU64::new(0)); // chardev doesn't use generations yet
    let state_ptr = core::ptr::from_mut::<VirtmcuChardevState>(&mut *state);

    match virtmcu_qom::sync::DeterministicReceiver::new(
        &*transport,
        &rx_topic,
        generation,
        state_ptr as *mut c_void,
        decode_chardev,
        deliver_chardev,
    ) {
        Ok(receiver) => {
            state.receiver = Some(receiver);
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

            state.timer_ptr.store(0, AtomicOrdering::Release);
            state.tx_timer_ptr.store(0, AtomicOrdering::Release);

            // Take the DeterministicReceiver to automatically undeclare and wait
            state.receiver.take();

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

            // Bridge handles tx_thread teardown and vcpu draining!
            drop(state);

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

#[used]
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

pub struct ChardevTransport {
    pub transport: Arc<dyn virtmcu_api::DataTransport>,
    pub topic: String,
    pub rx_out: Receiver<TxPacket>,
    pub tx_fifo: Arc<virtmcu_qom::sync::Mutex<VecDeque<u8>>>, // virtmcu-allow: mutex reasoning="TX FIFO managed securely"
    pub baud_delay_ns: Arc<AtomicU64>,
    pub tx_timer_ptr: Arc<AtomicUsize>,
}

unsafe impl Send for ChardevTransport {}
unsafe impl Sync for ChardevTransport {}

impl CoSimTransport for ChardevTransport {
    type Request = Vec<u8>;
    type Response = ();

    fn run_rx_loop(&self, ctx: &CoSimContext<Self::Response>) {
        let mut buffer = Vec::with_capacity(SEND_BUF_CAPACITY);
        let mut first_vtime = 0;
        let mut first_seq = 0;
        let mut last_send = std::time::Instant::now();

        while ctx.is_running() {
            match self.rx_out.recv_timeout(core::time::Duration::from_millis(RECV_TIMEOUT_MS)) {
                Ok(packet) => {
                    if buffer.is_empty() {
                        first_vtime = packet.vtime;
                        first_seq = packet.sequence;
                    }
                    buffer.extend_from_slice(&packet.data);
                    if buffer.len() >= FLUSH_THRESHOLD
                        || last_send.elapsed().as_millis() >= FLUSH_INTERVAL_MS as u128
                    {
                        send_packet(
                            &*self.transport,
                            &self.topic,
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
                            &*self.transport,
                            &self.topic,
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
    }

    fn send_request(&self, req: Self::Request) -> bool {
        let data = req;
        let mut fifo = self.tx_fifo.lock(); // virtmcu-allow: mutex reasoning="TX FIFO managed securely"
        let was_empty = fifo.is_empty();
        if fifo.len() + data.len() <= MAX_FIFO_SIZE {
            fifo.extend(data.iter().copied());
        } else {
            virtmcu_qom::sim_info!("TX FIFO overflow, dropping {} bytes", data.len());
        }

        if was_empty && !data.is_empty() {
            // SAFETY: Safe to query clock under BQL
            let now = unsafe {
                virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL)
            };
            let delay = self.baud_delay_ns.load(AtomicOrdering::Relaxed);
            let timer_ptr = self.tx_timer_ptr.load(AtomicOrdering::Acquire) as *mut QemuTimer;
            if !timer_ptr.is_null() {
                unsafe {
                    virtmcu_qom::timer::virtmcu_timer_mod(timer_ptr, now + delay as i64);
                }
            }
        }
        false
    }

    fn interrupt_rx(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chardev_virtmcu_layout() {
        const NODE_OFFSET: usize = 16;
        const HAS_MAX_BACKLOG_OFFSET: usize = 48;
        const HAS_BAUD_RATE_NS_OFFSET: usize = 49;
        const MAX_BACKLOG_OFFSET: usize = 56;
        const BAUD_RATE_NS_OFFSET: usize = 64;
        const OPTS_SIZE: usize = 72;
        const CHARDEV_SIZE: usize = 160;

        assert_eq!(core::mem::offset_of!(ChardevVirtmcuOptions, logfile), 0);
        assert_eq!(core::mem::offset_of!(ChardevVirtmcuOptions, node), NODE_OFFSET);
        assert_eq!(
            core::mem::offset_of!(ChardevVirtmcuOptions, has_max_backlog),
            HAS_MAX_BACKLOG_OFFSET
        );
        assert_eq!(
            core::mem::offset_of!(ChardevVirtmcuOptions, has_baud_rate_ns),
            HAS_BAUD_RATE_NS_OFFSET
        );
        assert_eq!(core::mem::offset_of!(ChardevVirtmcuOptions, max_backlog), MAX_BACKLOG_OFFSET);
        assert_eq!(core::mem::offset_of!(ChardevVirtmcuOptions, baud_rate_ns), BAUD_RATE_NS_OFFSET);
        assert_eq!(core::mem::size_of::<ChardevVirtmcuOptions>(), OPTS_SIZE);
        assert_eq!(core::mem::size_of::<Chardev>(), CHARDEV_SIZE);
    }
}

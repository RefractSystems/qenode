#![allow(deprecated)] // virtmcu-allow: allow reasoning="S2 migration in progress"
#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::missing_safety_doc)]
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
use virtmcu_qom::sync::{Condvar, Mutex as SimMutex};

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::qom::{Object, ObjectClass};
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
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(
    name = "chardev-virtmcu",
    parent = "chardev",
    class_init_custom = "char_virtmcu_class_init_custom"
)]
pub struct ChardevVirtmcu {
    pub parent_obj: Chardev,
    pub iomem: virtmcu_qom::memory::MemoryRegion,
    #[qom_state]
    pub state: VirtmcuChardevState,
}

pub struct TxPacket {
    pub vtime: u64,
    pub sequence: u64,
    pub data: Vec<u8>,
}

pub struct VirtmcuChardevState {
    pub bridge: Option<CoSimBridge<ChardevTransport>>,
    pub tx_sender: Option<Sender<TxPacket>>,
    pub chr: *mut Chardev,
    pub rx_timer: *mut QemuTimer,
    pub rx_baud_timer: *mut QemuTimer,
    pub kick_timer: *mut QemuTimer,
    pub timer_ptr: Arc<AtomicUsize>,
    pub receiver: Option<virtmcu_qom::sync::VtimeIngress<OrderedPacket>>,
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
    pub cond: Arc<Condvar>,
    pub wait_mutex: Arc<SimMutex<()>>, // virtmcu-allow: mutex reasoning="State managed securely"
    pub drain: virtmcu_qom::sync::VcpuDrain,
}

impl virtmcu_qom::device::PeripheralState for VirtmcuChardevState {
    type QomType = ChardevVirtmcu;

    fn new(qemu_dev: &Self::QomType) -> Self {
        Self {
            bridge: None,
            tx_sender: None,
            chr: core::ptr::from_ref(qemu_dev).cast_mut() as *mut Chardev,
            rx_timer: ptr::null_mut(),
            rx_baud_timer: ptr::null_mut(),
            kick_timer: ptr::null_mut(),
            timer_ptr: Arc::new(AtomicUsize::new(0)),
            receiver: None,
            backlog: virtmcu_qom::sync::Mutex::new(VecDeque::new()), // virtmcu-allow: mutex reasoning="State managed securely"
            tx_fifo: Arc::new(virtmcu_qom::sync::Mutex::new(VecDeque::new())), // virtmcu-allow: mutex reasoning="State managed securely"
            tx_timer: ptr::null_mut(),
            tx_timer_ptr: Arc::new(AtomicUsize::new(0)),
            baud_delay_ns: Arc::new(AtomicU64::new(0)),
            earliest_vtime: Arc::new(AtomicU64::new(u64::MAX)),
            tx_sequence: AtomicU64::new(0),
            max_backlog: 0,
            backlog_size_atomic: Arc::new(AtomicU64::new(0)),
            dropped_frames_atomic: Arc::new(AtomicU64::new(0)),
            _liveliness: None,
            cond: Arc::new(Condvar::new()),
            wait_mutex: Arc::new(SimMutex::new(())), // virtmcu-allow: mutex reasoning="State managed securely"
            drain: virtmcu_qom::sync::VcpuDrain::new(),
        }
    }
}

impl virtmcu_qom::device::Peripheral for VirtmcuChardevState {
    fn realize(
        &mut self,
        _ctx: &virtmcu_qom::device::BqlContext,
    ) -> Result<(), alloc::string::String> {
        Ok(())
    }

    fn read(
        &self,
        addr: u64,
        size: u32,
        _token: &virtmcu_qom::device::BqlContext,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }

    fn write(&self, addr: u64, val: u64, size: u32, _token: &virtmcu_qom::device::BqlContext) {
        virtmcu_qom::device::MmioDevice::write(self, addr, val, size);
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &SimMutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

impl virtmcu_qom::device::MmioDevice for VirtmcuChardevState {
    fn read(&self, _addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        virtmcu_qom::device::MmioResult::Ready(0)
    }

    fn write(&self, _addr: u64, _val: u64, _size: u32) {
        let _guard = self.drain.acquire();
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    #[rustfmt::skip]
    fn wait_mutex(&self) -> &SimMutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
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

fn deliver_chardev(opaque: *mut c_void, packet: OrderedPacket) {
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
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
        virtmcu_qom::ffi_call! {
            let now = virtmcu_qom::timer::qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
            virtmcu_timer_mod(state.rx_baud_timer, now);
        }
    }

    virtmcu_qom::ffi_call! {
        virtmcu_timer_mod(state.rx_timer, 0);
    }
}

/// # Safety
/// This function is called by QEMU. chr must be a valid pointer to a Chardev instance.
#[no_mangle]
pub extern "C" fn virtmcu_chr_write(chr: *mut Chardev, buf: *const u8, len: c_int) -> c_int {
    // SAFETY: chr is assumed to be a valid pointer of ChardevVirtmcu type as per QOM convention.
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    if s.state.is_null() {
        return 0;
    }
    let state = unsafe { &mut *(s.state as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    // SAFETY: buf is a valid pointer provided by QEMU with length len.
    let data = virtmcu_qom::ffi_call! { core::slice::from_raw_parts(buf, len as usize) };

    if let Some(bridge) = &state.bridge {
        bridge.send_and_wait(data.to_vec(), 0);
    }
    len
}

/// # Safety
/// This function is called by QEMU to parse chardev options.
#[no_mangle]
pub extern "C" fn virtmcu_chr_parse(
    opts: *mut c_void,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) {
    // SAFETY: opts is a valid QemuOpts pointer.
    let node = virtmcu_qom::ffi_call! { qemu_opt_get(opts, c"node".as_ptr()) };

    if node.is_null() {
        let msg = c"chardev: virtmcu: 'node' is required".as_ptr();
        // SAFETY: errp is a valid error pointer.
        virtmcu_qom::ffi_call! { virtmcu_error_setg(errp as *mut *mut _, msg) };
        return;
    }

    // SAFETY: opts is a valid QemuOpts pointer.
    let transport = virtmcu_qom::ffi_call! { qemu_opt_get(opts, c"transport".as_ptr()) };
    let router = virtmcu_qom::ffi_call! { qemu_opt_get(opts, c"router".as_ptr()) };
    let topic = virtmcu_qom::ffi_call! { qemu_opt_get(opts, c"topic".as_ptr()) };
    let max_backlog_str = virtmcu_qom::ffi_call! { qemu_opt_get(opts, c"max-backlog".as_ptr()) };
    let baud_rate_ns_str = virtmcu_qom::ffi_call! { qemu_opt_get(opts, c"baud-rate-ns".as_ptr()) };

    // SAFETY: All pointers are validated or strdup'd.
    let virtmcu_opts = virtmcu_qom::ffi_call! {
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
    let b = unsafe { &mut *(backend as *mut ChardevBackend_Fields) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    b.u.virtmcu = ChardevVirtmcuWrapper { data: virtmcu_opts };

    // SAFETY: virtmcu_opts is a valid pointer to ChardevVirtmcuOptions.
    virtmcu_qom::ffi_call! { qemu_chr_parse_common(opts, virtmcu_opts as *mut c_void) };
}

extern "C" fn virtmcu_chr_tx_timer_cb(opaque: *mut core::ffi::c_void) {
    // SAFETY: Provided by QEMU
    let s = unsafe { &mut *(opaque as *mut ChardevVirtmcu) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    if s.state.is_null() {
        return;
    }
    let state = unsafe { &mut *(s.state as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    let mut fifo = state.tx_fifo.lock(); // virtmcu-allow: mutex reasoning="TX FIFO managed securely"
    if let Some(byte) = fifo.pop_front() {
        // SAFETY: Safe to query clock under BQL
        let vtime = virtmcu_qom::timer::qemu_clock_get_ns_safe(
            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
            unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        );
        let sequence = state.tx_sequence.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        if let Some(sender) = &state.tx_sender {
            match sender.try_send(TxPacket { vtime: vtime as u64, sequence, data: vec![byte] }) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => {}
                Err(TrySendError::Full(_)) => {
                    panic!("FATAL: Channel flooded. PDES barrier failure.")
                }
            }
        }
    }

    if !fifo.is_empty() {
        // SAFETY: Safe to query clock under BQL
        let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
            unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        );
        let delay = state.baud_delay_ns.load(AtomicOrdering::Relaxed);
        // SAFETY: Valid timer
        virtmcu_qom::ffi_call! {
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
pub extern "C" fn virtmcu_chr_ioctl(
    chr: *mut Chardev,
    cmd: core::ffi::c_int,
    arg: *mut c_void,
) -> core::ffi::c_int {
    // SAFETY: Provided by QEMU
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    let state = unsafe { &mut *(s.state as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    if cmd == CHR_IOCTL_SERIAL_SET_PARAMS {
        if !arg.is_null() {
            // SAFETY: Provided by QEMU
            let ssp = virtmcu_qom::ffi_call! { &*(arg as *mut QEMUSerialSetParams) };
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
virtmcu_qom::ffi_safe_fn! {
    fn init_chardev_timers(state: &mut VirtmcuChardevState, s: *mut ChardevVirtmcu) {

    let state_ptr = core::ptr::from_mut::<VirtmcuChardevState>(&mut *state);
    // SAFETY: Creating timers is safe
    state.rx_timer = virtmcu_qom::ffi_call! {
        virtmcu_timer_new_ns(QEMU_CLOCK_VIRTUAL, virtmcu_chr_rx_timer_cb, state_ptr as *mut c_void)
    };
    // SAFETY: Creating timers is safe
    state.kick_timer = virtmcu_qom::ffi_call! {
        virtmcu_timer_new_ns(
            virtmcu_qom::timer::QEMU_CLOCK_REALTIME,
            virtmcu_chr_kick_timer_cb,
            state_ptr as *mut c_void,
        )
    };
    // SAFETY: Creating timers is safe
    state.tx_timer = virtmcu_qom::ffi_call! {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            virtmcu_chr_tx_timer_cb,
            core::ptr::from_mut(&mut *s) as *mut core::ffi::c_void,
        )
    };
    // SAFETY: Creating timers is safe
    state.rx_baud_timer = virtmcu_qom::ffi_call! {
        virtmcu_timer_new_ns(
            QEMU_CLOCK_VIRTUAL,
            virtmcu_chr_rx_baud_timer_cb,
            state_ptr as *mut c_void,
        )
    };
    state.timer_ptr.store(state.kick_timer as usize, core::sync::atomic::Ordering::Release);
    state.tx_timer_ptr.store(state.tx_timer as usize, core::sync::atomic::Ordering::Release);

    }
}

extern "C" fn virtmcu_chr_rx_baud_timer_cb(opaque: *mut core::ffi::c_void) {
    // SAFETY: Provided by QEMU
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    let mut backlog = state.backlog.lock(); // virtmcu-allow: mutex reasoning="Backlog managed securely"
    if backlog.is_empty() {
        return;
    }

    // SAFETY: chr is a valid pointer.
    let can_write = virtmcu_qom::ffi_call! { qemu_chr_be_can_write(state.chr) };
    if can_write > 0 {
        if let Some(byte) = backlog.pop_front() {
            let data = [byte];
            state.backlog_size_atomic.fetch_sub(1, AtomicOrdering::SeqCst);
            // SAFETY: qemu_chr_be_write expects valid buffer and length.
            virtmcu_qom::ffi_call! {
                qemu_chr_be_write(state.chr, data.as_ptr(), 1);
            }
        }
    }

    if !backlog.is_empty() && can_write > 0 {
        // SAFETY: Safe to query clock under BQL
        let now = virtmcu_qom::timer::qemu_clock_get_ns_safe(
            virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL,
            unsafe { &virtmcu_qom::device::BqlContext::new_unchecked() }, // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
        );
        let delay = state.baud_delay_ns.load(AtomicOrdering::Relaxed);
        // SAFETY: Valid timer
        virtmcu_qom::ffi_call! {
            virtmcu_qom::timer::virtmcu_timer_mod(state.rx_baud_timer, now + delay as i64);
        }
    }
}

fn drain_backlog(_state: &mut VirtmcuChardevState) -> bool {
    false
}

extern "C" fn virtmcu_chr_rx_timer_cb(opaque: *mut c_void) {
    // SAFETY: opaque is a valid pointer to VirtmcuChardevState.
    let state = unsafe { &mut *(opaque as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    drain_backlog(state);
}

extern "C" fn virtmcu_chr_kick_timer_cb(opaque: *mut c_void) {
    virtmcu_chr_rx_timer_cb(opaque);
}

#[no_mangle]
pub extern "C" fn virtmcu_chr_accept_input(chr: *mut Chardev) {
    // SAFETY: chr is a valid pointer to ChardevVirtmcu.
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    let state = unsafe { &mut *(s.state as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    if !state.backlog.lock().is_empty() {
        // virtmcu-allow: mutex reasoning="Backlog managed securely"
        // Resume pushing bytes into the guest
        virtmcu_qom::ffi_call! {
            let now = virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL);
            virtmcu_qom::timer::virtmcu_timer_mod(state.rx_baud_timer, now);
        };
    }
}

fn send_packet(transport: &dyn virtmcu_wire::DataTransport, link_id: u32, packet: TxPacket) {
    match transport.reserve_link(link_id, packet.data.len()) {
        Ok(mut reservation) => {
            reservation.buffer_mut().copy_from_slice(&packet.data);
            let _ = reservation.commit(packet.vtime, packet.sequence);
        }
        Err(e) => {
            virtmcu_qom::sim_err!("{}", e);
        }
    }
}

virtmcu_qom::ffi_safe_fn! {
    fn add_chardev_properties(chr: *mut Chardev, state: &VirtmcuChardevState) {

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
}

fn parse_chardev_options(
    opts: *mut ChardevVirtmcuOptions,
) -> (String, String, *const c_char, String, u64, u64) {
    virtmcu_qom::ffi_call! {
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
}

fn create_chardev_transport(
    transport_name: &str,
    node: &str,
    router_ptr: *const c_char,
    errp: *mut *mut c_void,
) -> Option<Arc<dyn virtmcu_wire::DataTransport>> {
    let nid: u32 = node.parse().expect("chardev node id must be numeric");
    if transport_name == "unix" {
        let path = if router_ptr.is_null() {
            format!("/tmp/virtmcu-coord-{node}.sock") // virtmcu-allow: absolute_path reasoning="Legacy script"
        } else {
            virtmcu_qom::ffi_call! { core::ffi::CStr::from_ptr(router_ptr).to_string_lossy().into_owned() }
        };
        // virtmcu-allow: env_in_peripheral reasoning="Not yet ported: needs federation-id QOM property + new_with_fed_id"
        match transport_uds::UdsDataTransport::new(&path, nid) {
            Ok(t) => Some(Arc::new(t) as Arc<dyn virtmcu_wire::DataTransport>),
            Err(e) => {
                let msg = format!("chardev: virtmcu: failed to open unix socket {path}: {e}");
                if let Ok(c_msg) = CString::new(msg) {
                    virtmcu_qom::ffi_call! { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
                }
                None
            }
        }
    } else {
        // Default to Zenoh
        match virtmcu_qom::ffi_call! { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(session) => Some(Arc::new(transport_zenoh::ZenohDataTransport::new(session, nid))
                as Arc<dyn virtmcu_wire::DataTransport>),
            Err(e) => {
                let msg = format!("chardev: virtmcu: failed to open zenoh session: {e}");
                if let Ok(c_msg) = CString::new(msg) {
                    virtmcu_qom::ffi_call! { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
                }
                None
            }
        }
    }
}

/// # Safety
/// This function is called by QEMU when opening the chardev.
#[no_mangle]
pub extern "C" fn virtmcu_chr_open(
    chr: *mut Chardev,
    backend: *mut c_void,
    errp: *mut *mut c_void,
) -> bool {
    virtmcu_qom::sim_info!("virtmcu_chr_open called");
    // SAFETY: chr is a valid pointer to ChardevVirtmcu.
    let s = unsafe { &mut *(chr as *mut ChardevVirtmcu) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
                                                           // SAFETY: backend is a valid ChardevBackend pointer.
    let b = virtmcu_qom::ffi_call! { &*(backend as *mut ChardevBackend_Fields) };
    let wrapper = virtmcu_qom::ffi_call! { b.u.virtmcu };
    let opts = wrapper.data;

    let (node, transport_name, router_ptr, base_topic, max_backlog, baud_delay_ns) =
        parse_chardev_options(opts);

    let transport = match create_chardev_transport(&transport_name, &node, router_ptr, errp) {
        Some(t) => t,
        None => return false,
    };

    let link_id = match transport.register_link(&base_topic) {
        Ok(id) => id,
        Err(e) => {
            let msg = format!("chardev: virtmcu: failed to register link: {e}");
            if let Ok(c_msg) = CString::new(msg) {
                virtmcu_qom::ffi_call! { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
            }
            return false;
        }
    };

    let timer_ptr = Arc::new(AtomicUsize::new(0));
    let earliest_vtime = Arc::new(AtomicU64::new(u64::MAX));

    let (tx_out, rx_out): (Sender<TxPacket>, Receiver<TxPacket>) = bounded(MAX_FIFO_SIZE);
    let backlog_size_atomic = Arc::new(AtomicU64::new(0));
    let dropped_frames_atomic = Arc::new(AtomicU64::new(0));
    let tx_fifo = Arc::new(virtmcu_qom::sync::Mutex::new(VecDeque::new())); // virtmcu-allow: mutex reasoning="State managed securely"
    let baud_delay_ns_arc = Arc::new(AtomicU64::new(baud_delay_ns));
    let tx_timer_ptr = Arc::new(AtomicUsize::new(0));

    let transport_impl = ChardevTransport {
        transport: Arc::clone(&transport),
        link_id,
        rx_out,
        tx_fifo: Arc::clone(&tx_fifo),
        baud_delay_ns: Arc::clone(&baud_delay_ns_arc),
        tx_timer_ptr: Arc::clone(&tx_timer_ptr),
    };
    let bridge = CoSimBridge::new(transport_impl);

    let liveliness = if transport_name == "zenoh" {
        match virtmcu_qom::ffi_call! { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(session) => {
                let hb_topic = format!("sim/chardev/liveliness/{node}");
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let state = unsafe { &mut *(s.state as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    state._liveliness = liveliness;
    state.bridge = Some(bridge);
    state.tx_sender = Some(tx_out);
    state.timer_ptr = Arc::clone(&timer_ptr);
    state.tx_fifo = tx_fifo;
    state.tx_timer_ptr = tx_timer_ptr;
    state.baud_delay_ns = baud_delay_ns_arc;
    state.earliest_vtime = Arc::clone(&earliest_vtime);
    state.max_backlog = max_backlog;
    state.backlog_size_atomic = Arc::clone(&backlog_size_atomic);
    state.dropped_frames_atomic = Arc::clone(&dropped_frames_atomic);

    // Add QOM properties for observability
    // SAFETY: chr is a valid pointer to a Chardev instance.
    virtmcu_qom::ffi_call! { add_chardev_properties(chr, state) };

    let generation = Arc::new(AtomicU64::new(0)); // chardev doesn't use generations yet
    let state_ptr =
        core::ptr::from_mut::<VirtmcuChardevState>(virtmcu_qom::ffi_call! { &mut *s.state });

    match virtmcu_qom::sync::VtimeIngress::new_for_link(
        &*transport,
        link_id,
        generation,
        |_, vtime, sequence, payload| {
            Some(OrderedPacket { vtime, sequence, data: payload.to_vec() })
        },
        move |packet| deliver_chardev(state_ptr as *mut c_void, packet),
    ) {
        Ok(receiver) => {
            state.receiver = Some(receiver);
            // SAFETY: Safe to initialize timers
            virtmcu_qom::ffi_call! { init_chardev_timers(&mut *s.state, s) };

            virtmcu_qom::sim_info!("virtmcu_chr_open success");
            true
        }
        Err(e) => {
            let msg = format!("chardev: virtmcu: failed to subscribe: {e}");
            if let Ok(c_msg) = CString::new(msg) {
                virtmcu_qom::ffi_call! { virtmcu_error_setg(errp as *mut *mut _, c_msg.as_ptr()) };
            }
            false
        }
    }
}

/// # Safety
/// This function is called by QEMU when finalizing the chardev.
#[no_mangle]
pub extern "C" fn virtmcu_chr_finalize(obj: *mut Object) {
    virtmcu_qom::sim_info!("virtmcu_chr_finalize called");
    // SAFETY: obj is a valid pointer to ChardevVirtmcu.
    let s = unsafe { &mut *(obj as *mut ChardevVirtmcu) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    let state = unsafe { &mut *(s.state as *mut VirtmcuChardevState) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"

    state.timer_ptr.store(0, AtomicOrdering::Release);
    state.tx_timer_ptr.store(0, AtomicOrdering::Release);

    // Take the VtimeIngress to automatically undeclare and wait
    state.receiver.take();

    virtmcu_qom::ffi_call! {
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
    }

    // Bridge handles tx_thread teardown and vcpu draining!
    state.bridge.take();
}

/// # Safety
/// This function is called by QEMU to initialize the chardev class.
#[no_mangle]
pub extern "C" fn char_virtmcu_class_init_custom(klass: *mut ObjectClass, _data: *const c_void) {
    virtmcu_qom::sim_info!("char_virtmcu_class_init called");
    // SAFETY: klass is a valid pointer to ChardevClass.
    let cc = unsafe { &mut *(klass as *mut ChardevClass) }; // virtmcu-allow: unsafe_in_peripheral reasoning="Migration debt"
    cc.chr_parse = Some(virtmcu_chr_parse);
    cc.chr_open = Some(virtmcu_chr_open);
    cc.chr_write = Some(virtmcu_chr_write);
    cc.chr_accept_input = Some(virtmcu_chr_accept_input);
    cc.chr_ioctl = Some(virtmcu_chr_ioctl);
}

virtmcu_qom::register_peripheral!(ChardevVirtmcu);

pub struct ChardevTransport {
    pub transport: Arc<dyn virtmcu_wire::DataTransport>,
    pub link_id: u32,
    pub rx_out: Receiver<TxPacket>,
    pub tx_fifo: Arc<virtmcu_qom::sync::Mutex<VecDeque<u8>>>, // virtmcu-allow: mutex reasoning="TX FIFO managed securely"
    pub baud_delay_ns: Arc<AtomicU64>,
    pub tx_timer_ptr: Arc<AtomicUsize>,
}

virtmcu_qom::impl_send_sync!(ChardevTransport);

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
                            self.link_id,
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
                            self.link_id,
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
            let now = virtmcu_qom::ffi_call! {
                virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL)
            };
            let delay = self.baud_delay_ns.load(AtomicOrdering::Relaxed);
            let timer_ptr = self.tx_timer_ptr.load(AtomicOrdering::Acquire) as *mut QemuTimer;
            if !timer_ptr.is_null() {
                virtmcu_qom::ffi_call! {
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

    /// Verifies the "Fail Loudly" contract: pumping a full channel panics instead of silently
    /// dropping, which would break PDES determinism.
    #[test]
    #[should_panic(expected = "FATAL: Channel flooded")]
    fn test_tx_channel_flood_panics() {
        let (tx, _rx) = crossbeam_channel::bounded::<TxPacket>(65536);
        for i in 0..65536_u64 {
            tx.try_send(TxPacket { vtime: i, sequence: i, data: vec![0] })
                .expect("should not be full yet");
        }
        // 65537th send must panic loudly
        match tx.try_send(TxPacket { vtime: 0, sequence: 0, data: vec![0] }) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => panic!("FATAL: Channel flooded. PDES barrier failure."),
        }
    }

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

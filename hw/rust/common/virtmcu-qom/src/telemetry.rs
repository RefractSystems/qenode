//! Enterprise Lock-Free Telemetry System.

#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
use alloc::format;
use core::sync::atomic::AtomicU32;
#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
use core::sync::atomic::Ordering;
#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
use crossbeam_channel::{bounded, Sender};
#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
use std::sync::OnceLock;
#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
use std::thread;

#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
extern "C" {
    static mut virtmcu_global_node_id: u32;
    static mut virtmcu_global_vtime_ns: u64;
}

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Global node ID for telemetry. Provided as a stub in tests.
pub static mut virtmcu_global_node_id: u32 = 0; // virtmcu-allow: static_state reasoning="Stub provided for native unit tests"

#[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
#[no_mangle]
/// Global virtual time for telemetry. Provided as a stub in tests.
pub static mut virtmcu_global_vtime_ns: u64 = 0; // virtmcu-allow: static_state reasoning="Stub provided for native unit tests"

/// Updates the global virtual time.
pub fn update_global_vtime(vtime_ns: u64) {
    unsafe {
        core::ptr::write_volatile(&raw mut virtmcu_global_vtime_ns, vtime_ns);
    }
}

/// Updates the global node ID.
pub fn update_global_node_id(node_id: u32) {
    unsafe {
        core::ptr::write_volatile(&raw mut virtmcu_global_node_id, node_id);
    }
}

/// Returns the current global virtual time in nanoseconds.
pub fn get_global_vtime() -> u64 {
    unsafe { core::ptr::read_volatile(&raw const virtmcu_global_vtime_ns) }
}

/// Returns the current global node ID.
pub fn get_global_node_id() -> u32 {
    unsafe { core::ptr::read_volatile(&raw const virtmcu_global_node_id) }
}

/// Number of logs dropped due to queue overflow.
pub static DROPPED_LOGS: AtomicU32 = AtomicU32::new(0); // virtmcu-allow: static_state reasoning="Global metric accumulator"

#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
const LOG_QUEUE_SIZE: usize = 4096;
#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
const VTIME_WIDTH: usize = 10;
#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
const VTIME_PRECISION: usize = 2;

#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
static LOG_CHANNEL: OnceLock<Sender<LogEntry>> = OnceLock::new(); // virtmcu-allow: static_state reasoning="Safely exported channel"

/// Severity level of the log entry.
#[repr(u8)]
#[derive(Copy, Clone, Debug)]
pub enum LogLevel {
    /// Trace
    Trace = 0,
    /// Debug
    Debug = 1,
    /// Info
    Info = 2,
    /// Warn
    Warn = 3,
    /// Error
    Error = 4,
}

impl LogLevel {
    /// Returns the string representation of the log level.
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO ",
            LogLevel::Warn => "WARN ",
            LogLevel::Error => "ERROR",
        }
    }
}

/// A structured log entry.
pub struct LogEntry {
    /// Virtual time in nanoseconds
    pub vtime: u64,
    /// Node ID
    pub node_id: u32,
    /// Severity level
    pub level: LogLevel,
    /// Module name
    pub module: &'static str,
    /// Formatted message buffer
    pub msg: [u8; 512],
    /// Length of the formatted message
    pub msg_len: usize,
}

/// Log an error message.
#[macro_export]
macro_rules! sim_err {
    ($($arg:tt)*) => {{
        $crate::telemetry::sim_log($crate::telemetry::LogLevel::Error, module_path!(), format_args!($($arg)*));
    }};
}

/// Log a warning message.
#[macro_export]
macro_rules! sim_warn {
    ($($arg:tt)*) => {{
        $crate::telemetry::sim_log($crate::telemetry::LogLevel::Warn, module_path!(), format_args!($($arg)*));
    }};
}

/// Log an info message.
#[macro_export]
macro_rules! sim_info {
    ($($arg:tt)*) => {{
        $crate::telemetry::sim_log($crate::telemetry::LogLevel::Info, module_path!(), format_args!($($arg)*));
    }};
}

/// Log a debug message.
#[macro_export]
macro_rules! sim_debug {
    ($($arg:tt)*) => {{
        $crate::telemetry::sim_log($crate::telemetry::LogLevel::Debug, module_path!(), format_args!($($arg)*));
    }};
}

/// Log a trace message.
#[macro_export]
macro_rules! sim_trace {
    ($($arg:tt)*) => {{
        $crate::telemetry::sim_log($crate::telemetry::LogLevel::Trace, module_path!(), format_args!($($arg)*));
    }};
}

#[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
fn init_logger_thread() -> Sender<LogEntry> {
    let (tx, rx) = bounded::<LogEntry>(LOG_QUEUE_SIZE);
    thread::Builder::new()
        .name("virtmcu-logger".into())
        .spawn(move || {
            while let Ok(entry) = rx.recv() {
                let dropped = DROPPED_LOGS.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    // Use sim_warn! to log overflow. In the logger thread, this will
                    // add a message to the queue we are currently draining.
                    sim_warn!("Logger queue overflow: dropped {dropped} messages");
                }

                let msg_str = entry.msg.get(..entry.msg_len).map_or("<missing msg>", |b| {
                    core::str::from_utf8(b).unwrap_or("<invalid utf8>")
                });

                let vtime_ms = entry.vtime as f64 / 1_000_000.0;

                let formatted = format!(
                    "[VTime: {:>width$.prec$ } ms] [Node: {}] [{}] [{}] {}\n",
                    vtime_ms,
                    entry.node_id,
                    entry.level.as_str(),
                    entry.module,
                    msg_str,
                    width = VTIME_WIDTH,
                    prec = VTIME_PRECISION
                );
                unsafe {
                    crate::virtmcu_log(formatted.as_ptr() as *const _);
                }
            }
        })
        .expect("failed to spawn logger thread");
    tx
}

/// Primary entry point for simulation logging.
pub fn sim_log(level: LogLevel, module: &'static str, args: core::fmt::Arguments) {
    #[cfg(any(test, miri, feature = "standalone", virtmcu_unit_test))]
    {
        let _ = (level, module, args);
    }

    #[cfg(not(any(test, miri, feature = "standalone", virtmcu_unit_test)))]
    {
        let tx = LOG_CHANNEL.get_or_init(init_logger_thread);

        let mut entry = LogEntry {
            vtime: get_global_vtime(),
            node_id: get_global_node_id(),
            level,
            module,
            msg: [0u8; 512],
            msg_len: 0,
        };

        let mut cursor = crate::BufCursor::new(&mut entry.msg);
        let _ = core::fmt::write(&mut cursor, args);
        entry.msg_len = cursor.pos();

        if let Err(crossbeam_channel::TrySendError::Full(_)) = tx.try_send(entry) {
            DROPPED_LOGS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

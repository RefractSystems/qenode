//! Enterprise Lock-Free Telemetry System.

#[cfg(not(miri))]
use core::fmt::Write;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
#[cfg(not(miri))]
use crossbeam_channel::{bounded, Sender};
#[cfg(not(miri))]
use std::sync::OnceLock;
#[cfg(not(miri))]
use std::thread;

/// Global Node ID for this QEMU process.
pub static GLOBAL_NODE_ID: AtomicU32 = AtomicU32::new(0);
/// Global Virtual Time in nanoseconds.
pub static GLOBAL_VTIME: AtomicU64 = AtomicU64::new(0);
/// Number of logs dropped due to queue overflow.
pub static DROPPED_LOGS: AtomicU32 = AtomicU32::new(0);

#[cfg(not(miri))]
static LOG_CHANNEL: OnceLock<Sender<LogEntry>> = OnceLock::new();

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

/// Updates the global virtual time.
pub fn update_global_vtime(vtime_ns: u64) {
    GLOBAL_VTIME.store(vtime_ns, Ordering::Release);
}

/// Updates the global node ID.
pub fn update_global_node_id(node_id: u32) {
    let _ = GLOBAL_NODE_ID.compare_exchange(0, node_id, Ordering::Relaxed, Ordering::Relaxed);
}

#[cfg(not(miri))]
fn init_logger_thread() -> Sender<LogEntry> {
    let (tx, rx) = bounded::<LogEntry>(4096);
    thread::Builder::new()
        .name("virtmcu-logger".into())
        .spawn(move || {

            while let Ok(entry) = rx.recv() {
                let dropped = DROPPED_LOGS.swap(0, Ordering::Relaxed);
                if dropped > 0 {
                    let warning = format!(
                        "[VTime: SYSTEM    ] [Node: -] [WARN ] [telemetry] Logger queue overflow: dropped {dropped} messages\n\0"
                    );
                    // SAFETY: warning string is null-terminated.
                    #[cfg(not(miri))]
                    unsafe {
                        crate::virtmcu_log(warning.as_ptr() as *const _);
                    }
                    #[cfg(miri)]
                    eprintln!("{}", warning.trim_end_matches('\0'));
                    }

                    let msg_str =
                    core::str::from_utf8(&entry.msg[..entry.msg_len]).unwrap_or("<invalid utf8>");

                    let vtime_ms = entry.vtime as f64 / 1_000_000.0;

                    let formatted = format!(
                    "[VTime: {:>10.2} ms] [Node: {}] [{}] [{}] {}\n\0",
                    vtime_ms,
                    entry.node_id,
                    entry.level.as_str(),
                    entry.module,
                    msg_str
                    );

                    // SAFETY: formatted is null-terminated and valid for the duration of the call.
                    #[cfg(not(miri))]
                    unsafe {
                    crate::virtmcu_log(formatted.as_ptr() as *const _);
                    }
                    #[cfg(miri)]
                    eprintln!("{}", formatted.trim_end_matches('\0'));
            }
        })
        .unwrap_or_else(|_| std::process::abort());
    tx
}

#[doc(hidden)]
pub fn sim_log(level: LogLevel, module: &'static str, args: core::fmt::Arguments) {
    #[cfg(miri)]
    {
        let node_id = GLOBAL_NODE_ID.load(Ordering::Relaxed);
        eprintln!("[Node: {}] [{}] [{}] {}", node_id, level.as_str(), module, args);
        return;
    }

    #[cfg(not(miri))]
    {
        let tx = LOG_CHANNEL.get_or_init(init_logger_thread);

        let mut entry = LogEntry {
            vtime: GLOBAL_VTIME.load(Ordering::Acquire),
            node_id: GLOBAL_NODE_ID.load(Ordering::Relaxed),
            level,
            module,
            msg: [0; 512],
            msg_len: 0,
        };

        let mut cursor = crate::BufCursor::new(&mut entry.msg);
        let _ = write!(cursor, "{args}");
        entry.msg_len = cursor.pos();

        if tx.try_send(entry).is_err() {
            DROPPED_LOGS.fetch_add(1, Ordering::Relaxed);
        }
    }
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

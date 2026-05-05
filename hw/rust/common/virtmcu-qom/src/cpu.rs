use core::ffi::c_int;

#[repr(C, align(16))]
/// A struct
pub struct CPUState {
    /// A struct field
    pub parent_obj: crate::qom::Object,
    _padding1: [u8; 816 - 40], // Pad to cpu_index
    /// A struct field
    pub cpu_index: c_int,
    /// A struct field
    pub _opaque: [u8; 16624 - 816 - 4], // Pad to 16624
}

#[repr(C)]
/// A struct
pub struct VirtmcuQuantumTiming {
    /// A struct field
    pub quantum_start_vtime_ns: i64,
    /// A struct field
    pub quantum_delta_ns: i64,
    /// A struct field
    pub mujoco_time_ns: i64,
}

extern "C" {
    /// Helper implemented in C to call cpu_exit() on all CPUs.
    /// This avoids having to replicate QEMU's CPU_FOREACH macro in Rust.
    pub fn virtmcu_cpu_exit_all();

    /// A static
    pub static mut virtmcu_tcg_quantum_hook: Option<extern "C" fn(cpu: *mut CPUState)>;

    /// A setter
    #[link_name = "virtmcu_cpu_set_halt_hook"]
    fn qemu_virtmcu_cpu_set_halt_hook(cb: Option<extern "C" fn(cpu: *mut CPUState, halted: bool)>);
    /// A setter
    pub fn virtmcu_cpu_set_tcg_hook(cb: Option<extern "C" fn(cpu: *mut CPUState)>);
    /// A static
    pub static mut virtmcu_get_quantum_timing:
        Option<extern "C" fn(timing: *mut VirtmcuQuantumTiming)>;

    /// A function
    pub fn cpu_exit(cpu: *mut CPUState);
}

use std::sync::Mutex;
static HALT_HOOKS: Mutex<Vec<extern "C" fn(cpu: *mut CPUState, halted: bool)>> =
    Mutex::new(Vec::new());

/// Register a new CPU halt hook.
/// This allows multiple devices to observe halt events.
pub fn virtmcu_cpu_set_halt_hook(cb: Option<extern "C" fn(cpu: *mut CPUState, halted: bool)>) {
    let Some(cb) = cb else { return };
    let mut hooks = HALT_HOOKS.lock().unwrap();
    if hooks.is_empty() {
        unsafe {
            qemu_virtmcu_cpu_set_halt_hook(Some(multiplexed_halt_hook));
        }
    }
    hooks.push(cb);
}

extern "C" fn multiplexed_halt_hook(cpu: *mut CPUState, halted: bool) {
    let hooks = HALT_HOOKS.lock().unwrap();
    for hook in hooks.iter() {
        hook(cpu, halted);
    }
}

const _: () = assert!(core::mem::size_of::<CPUState>() == 16624);
const _: () = assert!(core::mem::offset_of!(CPUState, cpu_index) == 816);

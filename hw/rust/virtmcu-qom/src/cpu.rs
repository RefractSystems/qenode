use core::ffi::c_void;

#[repr(C)]
pub struct CPUState {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct VirtmcuQuantumTiming {
    pub quantum_start_vtime_ns: i64,
    pub quantum_delta_ns: i64,
    pub mujoco_time_ns: i64,
}

extern "C" {
    /// Helper implemented in C to call cpu_exit() on all CPUs.
    /// This avoids having to replicate QEMU's CPU_FOREACH macro in Rust.
    pub fn virtmcu_cpu_exit_all();

    pub static mut virtmcu_tcg_quantum_hook: Option<extern "C" fn(cpu: *mut CPUState)>;
    pub static mut virtmcu_get_quantum_timing: Option<extern "C" fn(timing: *mut VirtmcuQuantumTiming)>;

    pub fn cpu_exit(cpu: *mut CPUState);
}

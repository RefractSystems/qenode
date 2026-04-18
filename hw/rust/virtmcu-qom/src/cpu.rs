use core::ffi::c_int;

#[repr(C, align(16))]
pub struct CPUState {
    pub parent_obj: crate::qom::Object,
    _padding1: [u8; 816 - 40], // Pad to cpu_index
    pub cpu_index: c_int,
    pub _opaque: [u8; 16624 - 816 - 4], // Pad to 16624
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
    pub static mut virtmcu_cpu_halt_hook: Option<extern "C" fn(cpu: *mut CPUState, halted: bool)>;
    pub static mut virtmcu_get_quantum_timing:
        Option<extern "C" fn(timing: *mut VirtmcuQuantumTiming)>;

    pub fn cpu_exit(cpu: *mut CPUState);
}

const _: () = assert!(core::mem::size_of::<CPUState>() == 16624);
const _: () = assert!(core::mem::offset_of!(CPUState, cpu_index) == 816);

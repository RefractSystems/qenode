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
    pub absolute_vtime_ns: i64,
}

extern "C" {
    /// Helper implemented in C to call cpu_exit() on all CPUs.
    /// This avoids having to replicate QEMU's CPU_FOREACH macro in Rust.
    pub fn virtmcu_cpu_exit_all();

    /// A static
    pub static mut virtmcu_tcg_quantum_hook: Option<extern "C" fn(cpu: *mut CPUState)>;

    /// A setter
    pub fn virtmcu_cpu_set_tcg_hook(cb: Option<extern "C" fn(cpu: *mut CPUState)>);
    /// A static
    pub static mut virtmcu_get_quantum_timing:
        Option<extern "C" fn(timing: *mut VirtmcuQuantumTiming)>;

    /// A function
    pub fn cpu_exit(cpu: *mut CPUState);

    /// The multiplexed halt hooks array in QEMU (from qemu-multiple-halt-hooks.patch)
    #[link_name = "virtmcu_cpu_halt_hooks"]
    pub static mut VIRTMCU_CPU_HALT_HOOKS:
        [Option<extern "C" fn(cpu: *mut CPUState, halted: bool)>; 8];
}

/// Register a new CPU halt hook.
/// This allows multiple devices to observe halt events.
pub fn virtmcu_cpu_set_halt_hook(cb: Option<extern "C" fn(cpu: *mut CPUState, halted: bool)>) {
    if let Some(cb) = cb {
        unsafe {
            // Find an empty slot or a duplicate
            let mut i = 0;
            while i < 8 {
                if let Some(h) = VIRTMCU_CPU_HALT_HOOKS[i] {
                    if h as *const () == cb as *const () {
                        return;
                    }
                } else {
                    VIRTMCU_CPU_HALT_HOOKS[i] = Some(cb);
                    return;
                }
                i += 1;
            }
            // If we reach here, we are out of slots.
            // This is a catastrophic failure for the simulation.
            std::process::abort();
        }
    }
}

const _: () = assert!(core::mem::size_of::<CPUState>() == 16624);
const _: () = assert!(core::mem::offset_of!(CPUState, cpu_index) == 816);

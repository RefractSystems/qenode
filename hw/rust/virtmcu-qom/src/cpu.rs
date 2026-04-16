extern "C" {
    /// Helper implemented in C to call cpu_exit() on all CPUs.
    /// This avoids having to replicate QEMU's CPU_FOREACH macro in Rust.
    pub fn virtmcu_cpu_exit_all();
}

extern "C" {
    /// A function
    pub fn virtmcu_icount_enabled() -> bool;
    /// A function
    pub fn virtmcu_icount_advance(delta: i64);
}

/// A function
pub fn icount_enabled() -> bool {
    // SAFETY: This is a safe wrapper around a QEMU-provided FFI function that
    // just returns a boolean flag. It has no side effects and requires no
    // specific state beyond what QEMU guarantees at runtime.
    unsafe { virtmcu_icount_enabled() }
}

/// A function
pub fn icount_advance(delta: i64) {
    // SAFETY: This is a safe wrapper around a QEMU-provided FFI function.
    // It is used to manually advance the instruction counter when TCG is not
    // doing it automatically. QEMU handles the internal state safely.
    unsafe { virtmcu_icount_advance(delta) }
}

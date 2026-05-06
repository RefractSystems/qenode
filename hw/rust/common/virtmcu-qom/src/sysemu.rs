//! Safe wrappers for QEMU system emulation state.

#[cfg(not(any(test, miri, feature = "standalone")))]
extern "C" {
    /// A function
    pub fn virtmcu_runstate_is_running() -> bool;
}

/// A function
pub fn runstate_is_running() -> bool {
    #[cfg(not(any(test, miri, feature = "standalone")))]
    // SAFETY: virtmcu_runstate_is_running is a safe wrapper around QEMU's
    // runstate_is_running() which just checks a global state variable.
    unsafe { virtmcu_runstate_is_running() }

    #[cfg(any(test, miri, feature = "standalone"))]
    true
}

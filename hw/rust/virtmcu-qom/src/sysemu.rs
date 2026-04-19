//! Safe wrappers for QEMU system emulation state.

extern "C" {
    pub fn virtmcu_runstate_is_running() -> bool;
}

pub fn runstate_is_running() -> bool {
    unsafe { virtmcu_runstate_is_running() }
}

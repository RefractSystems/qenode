extern "C" {
    pub fn virtmcu_icount_enabled() -> bool;
    pub fn virtmcu_icount_advance(delta: i64);
}

pub fn icount_enabled() -> bool {
    unsafe { virtmcu_icount_enabled() }
}

pub fn icount_advance(delta: i64) {
    unsafe { virtmcu_icount_advance(delta) }
}

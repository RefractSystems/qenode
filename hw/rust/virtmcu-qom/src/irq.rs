#[repr(C)]
pub struct IRQState {
    _opaque: [u8; 0],
}

#[allow(non_camel_case_types)]
pub type qemu_irq = *mut IRQState;

#[derive(Copy, Clone)]
pub struct SafeIrq(pub qemu_irq);
unsafe impl Send for SafeIrq {}
unsafe impl Sync for SafeIrq {}

extern "C" {
    pub fn qemu_set_irq(irq: qemu_irq, level: i32);
    pub static mut virtmcu_irq_hook: Option<
        extern "C" fn(opaque: *mut core::ffi::c_void, n: core::ffi::c_int, level: core::ffi::c_int),
    >;
}

use core::ffi::{c_char, c_void};

#[repr(C)]
pub struct Chardev {
    _opaque: [u8; 0],
}

extern "C" {
    pub fn qemu_chr_be_write(s: *mut Chardev, buf: *const u8, len: usize);
}

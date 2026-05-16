use core::ffi::{c_char, c_int, c_uint, c_void};

pub type QemuPluginId = u64;

#[repr(C)]
pub struct QemuPluginTb {
    _private: [u8; 0],
}

#[repr(C)]
pub struct QemuPluginInsn {
    _private: [u8; 0],
}

#[repr(C)]
pub struct QemuInfo {
    pub target_name: *const c_char,
    pub version: QemuInfoVersion,
    pub system_emulation: bool,
    pub system: QemuInfoSystem,
}

#[repr(C)]
pub struct QemuInfoVersion {
    pub min: u32,
    pub cur: u32,
}

#[repr(C)]
pub struct QemuInfoSystem {
    pub smp_vcpus: c_int,
    pub max_vcpus: c_int,
}

#[repr(C)]
#[allow(clippy::enum_variant_names, dead_code)] // virtmcu-allow: allow reasoning="Plugin C API, variants constructed by QEMU"
pub enum QemuPluginCbFlags {
    NoRegs = 0,
    RRegs = 1,
    RwRegs = 2,
}

pub type QemuPluginVcpuTbTransCb = extern "C" fn(id: QemuPluginId, tb: *mut QemuPluginTb);
pub type QemuPluginVcpuUdataCb = extern "C" fn(vcpu_index: c_uint, userdata: *mut c_void);
pub type QemuPluginAtexitCb = extern "C" fn(id: QemuPluginId, userdata: *mut c_void);

extern "C" {
    /// Write a message through QEMU's own logging subsystem.
    /// Safe to call from `qemu_plugin_install` before any subscriber is initialized.
    pub fn qemu_plugin_outs(string: *const c_char);
    pub fn qemu_plugin_register_vcpu_tb_trans_cb(id: QemuPluginId, cb: QemuPluginVcpuTbTransCb);
    pub fn qemu_plugin_register_atexit_cb(
        id: QemuPluginId,
        cb: QemuPluginAtexitCb,
        userdata: *mut c_void,
    );
    pub fn qemu_plugin_tb_n_insns(tb: *const QemuPluginTb) -> usize;
    pub fn qemu_plugin_tb_get_insn(tb: *const QemuPluginTb, idx: usize) -> *mut QemuPluginInsn;
    pub fn qemu_plugin_insn_vaddr(insn: *const QemuPluginInsn) -> u64;
    pub fn qemu_plugin_insn_disas(insn: *const QemuPluginInsn) -> *mut c_char;
    pub fn qemu_plugin_register_vcpu_insn_exec_cb(
        insn: *mut QemuPluginInsn,
        cb: QemuPluginVcpuUdataCb,
        flags: QemuPluginCbFlags,
        userdata: *mut c_void,
    );
}

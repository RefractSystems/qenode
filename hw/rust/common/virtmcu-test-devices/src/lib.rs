// std is required: virtmcu-qom dependency brings in std
//! Test devices for VirtMCU simulation.

use core::ffi::{c_int, c_void};
use virtmcu_qom::chardev::{qemu_chr_fe_set_handlers, CharFrontend};
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
use virtmcu_qom::ssi::{SSIPeripheral, TYPE_SSI_PERIPHERAL};
use virtmcu_qom::{define_prop_chr, device_class, ssi_peripheral_class};

/* ── SPI Echo Device ──────────────────────────────────────────────────────── */

/// SPI Echo device structure
#[repr(C)]
pub struct SPIEcho {
    /// Parent object
    pub parent: SSIPeripheral,
}

unsafe extern "C" fn spi_echo_transfer(_dev: *mut SSIPeripheral, val: u32) -> u32 {
    // Simply echo back the received value
    val
}

unsafe extern "C" fn spi_echo_realize(_dev: *mut SSIPeripheral, _errp: *mut *mut c_void) {
    // Dummy realize to prevent QEMU from calling a NULL pointer
}

unsafe extern "C" fn spi_echo_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let spc = ssi_peripheral_class!(klass);
    (*spc).realize = Some(spi_echo_realize);
    (*spc).transfer = Some(spi_echo_transfer);
}

#[used]
static SPI_ECHO_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"spi-echo".as_ptr(),
    parent: TYPE_SSI_PERIPHERAL,
    instance_size: core::mem::size_of::<SPIEcho>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::ssi::SSIPeripheralClass>(),
    class_init: Some(spi_echo_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

/* ── UART Echo Device ─────────────────────────────────────────────────────── */

/// UART Echo device structure
#[repr(C)]
pub struct UARTEcho {
    /// Parent object
    pub parent_obj: virtmcu_qom::qdev::SysBusDevice,
    /// Char frontend for UART communication
    pub chr: CharFrontend,
}

unsafe extern "C" fn uart_echo_can_receive(_opaque: *mut c_void) -> c_int {
    128
}

unsafe extern "C" fn uart_echo_receive(opaque: *mut c_void, buf: *const u8, size: c_int) {
    let s = &mut *(opaque as *mut UARTEcho);
    // Echo back to the same frontend
    virtmcu_qom::chardev::qemu_chr_fe_write(&mut s.chr, buf, size);
}

unsafe extern "C" fn uart_echo_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut UARTEcho);
    qemu_chr_fe_set_handlers(
        &mut s.chr,
        Some(uart_echo_can_receive),
        Some(uart_echo_receive),
        None,
        None,
        dev,
        core::ptr::null_mut(),
        true,
    );
}

static UART_ECHO_PROPERTIES: [virtmcu_qom::qom::Property; 2] = [
    define_prop_chr!(c"chardev".as_ptr(), UARTEcho, chr),
    // SAFETY: QEMU expects a zeroed Property as a sentinel at the end of the array.
    unsafe { core::mem::zeroed() },
];

unsafe extern "C" fn uart_echo_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(uart_echo_realize);
    (*dc).user_creatable = true;
    virtmcu_qom::qdev::device_class_set_props_n(dc, UART_ECHO_PROPERTIES.as_ptr(), 1);
}

#[used]
static UART_ECHO_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"uart-echo".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<UARTEcho>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(uart_echo_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

/* ── Registration ─────────────────────────────────────────────────────────── */

/// Initialize all test device types
pub extern "C" fn test_devices_type_init() {
    // SAFETY: type_register_static is safe with valid TypeInfo pointers.
    unsafe {
        virtmcu_qom::qom::type_register_static(&raw const SPI_ECHO_TYPE_INFO);
        virtmcu_qom::qom::type_register_static(&raw const UART_ECHO_TYPE_INFO);
    }
}

// Use a custom DSO init pointer to register both types
#[used]
#[cfg_attr(target_os = "linux", link_section = ".init_array")]
/// DSO initialization pointer
pub static DSO_INIT_PTR: extern "C" fn() = {
    extern "C" fn wrapper() {
        #[cfg(not(miri))]
        {
            // SAFETY: register_dso_module_init is a QEMU-provided function.
            unsafe {
                virtmcu_qom::qom::register_dso_module_init(
                    test_devices_type_init,
                    virtmcu_qom::qom::MODULE_INIT_QOM,
                );
            }
        }
    }
    wrapper
};

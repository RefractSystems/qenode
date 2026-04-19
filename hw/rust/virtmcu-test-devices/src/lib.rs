#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_int, c_void};
use virtmcu_qom::chardev::{qemu_chr_fe_set_handlers, CharFrontend};
use virtmcu_qom::qdev::DeviceClass;
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
use virtmcu_qom::ssi::{SSIPeripheral, SSIPeripheralClass, TYPE_SSI_PERIPHERAL};
use virtmcu_qom::{define_prop_chr, define_properties, device_class, ssi_peripheral_class};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

/* ── SPI Echo Device ──────────────────────────────────────────────────────── */

#[repr(C)]
pub struct SPIEcho {
    pub parent: SSIPeripheral,
}

unsafe extern "C" fn spi_echo_transfer(_dev: *mut SSIPeripheral, val: u32) -> u32 {
    // Simply echo back the received value
    val
}

unsafe extern "C" fn spi_echo_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let spc = ssi_peripheral_class!(klass);
    (*spc).transfer = Some(spi_echo_transfer);
}

static SPI_ECHO_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"spi-echo".as_ptr(),
    parent: TYPE_SSI_PERIPHERAL,
    instance_size: core::mem::size_of::<SPIEcho>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<SSIPeripheralClass>(),
    class_init: Some(spi_echo_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

/* ── UART Echo Device ─────────────────────────────────────────────────────── */

#[repr(C)]
pub struct UARTEcho {
    pub parent: virtmcu_qom::qdev::SysBusDevice,
    pub chr: CharFrontend,
}

unsafe extern "C" fn uart_echo_can_receive(_opaque: *mut c_void) -> c_int {
    1024
}

unsafe extern "C" fn uart_echo_receive(opaque: *mut c_void, buf: *const u8, size: c_int) {
    let s = &mut *(opaque as *mut UARTEcho);
    // Echo back to the same chardev
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

define_properties!(
    UART_ECHO_PROPS,
    [define_prop_chr!(c"chardev".as_ptr(), UARTEcho, chr),]
);

unsafe extern "C" fn uart_echo_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(uart_echo_realize);
    virtmcu_qom::device_class_set_props!(dc, UART_ECHO_PROPS);
}

static UART_ECHO_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"uart-echo".as_ptr(),
    parent: virtmcu_qom::qdev::TYPE_SYS_BUS_DEVICE,
    instance_size: core::mem::size_of::<UARTEcho>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<DeviceClass>(),
    class_init: Some(uart_echo_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

/* ── Registration ─────────────────────────────────────────────────────────── */

#[no_mangle]
pub extern "C" fn test_devices_type_init() {
    unsafe {
        virtmcu_qom::qom::type_register_static(&SPI_ECHO_TYPE_INFO);
        virtmcu_qom::qom::type_register_static(&UART_ECHO_TYPE_INFO);
    }
}

// Use a custom DSO init pointer to register both types
#[used]
#[allow(non_upper_case_globals)]
#[cfg_attr(target_os = "linux", link_section = ".init_array")]
pub static __DSO_INIT_PTR: extern "C" fn() = {
    extern "C" fn wrapper() {
        unsafe {
            virtmcu_qom::qom::register_dso_module_init(
                test_devices_type_init,
                virtmcu_qom::qom::MODULE_INIT_QOM,
            );
        }
    }
    wrapper
};

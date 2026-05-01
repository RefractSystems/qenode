// std is required: virtmcu-qom dependency brings in std
#![allow(missing_docs)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_int, c_uint, c_void};
use virtmcu_qom::chardev::CharFrontend;
use virtmcu_qom::memory::{
    MemoryRegion, MemoryRegionImplRange, MemoryRegionOps, MemoryRegionValidRange,
};
use virtmcu_qom::qdev::{MACAddr, SysBusDevice};
use virtmcu_qom::qom::{ObjectClass, TypeInfo};
use virtmcu_qom::ssi::{SSIPeripheral, SSIPeripheralClass, TYPE_SSI_PERIPHERAL};
use virtmcu_qom::{declare_device_type, define_prop_chr, define_properties, device_class};

/* ── Common MMIO Helpers ─────────────────────────────────────────────────── */

static DUMMY_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(dummy_read),
    write: Some(dummy_write),
    read_with_attrs: core::ptr::null(),
    write_with_attrs: core::ptr::null(),
    endianness: 2, // DEVICE_LITTLE_ENDIAN
    _padding1: [0; 4],
    valid: MemoryRegionValidRange {
        min_access_size: 1,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
        accepts: core::ptr::null(),
    },
    impl_: MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
    },
};

unsafe extern "C" fn dummy_read(_opaque: *mut c_void, _addr: u64, _size: c_uint) -> u64 {
    0
}

unsafe extern "C" fn dummy_write(_opaque: *mut c_void, _addr: u64, _val: u64, _size: c_uint) {}

/* ── SPI Echo Device ──────────────────────────────────────────────────────── */

#[repr(C)]
pub struct SPIEcho {
    pub parent_obj: SSIPeripheral,
}

unsafe extern "C" fn spi_echo_realize(_dev: *mut SSIPeripheral, _errp: *mut *mut c_void) {
    // Nothing to do for realization
}

unsafe extern "C" fn spi_echo_transfer(_dev: *mut SSIPeripheral, val: u32) -> u32 {
    val
}

unsafe extern "C" fn spi_echo_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let ssc = klass as *mut SSIPeripheralClass;
    (*ssc).realize = Some(spi_echo_realize);
    (*ssc).transfer = Some(spi_echo_transfer);
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

declare_device_type!(SPI_ECHO_TYPE_INIT, SPI_ECHO_TYPE_INFO);

/* ── UART Echo Device ─────────────────────────────────────────────────────── */

#[repr(C)]
pub struct UARTEcho {
    pub parent: SysBusDevice,
    pub chr: CharFrontend,
    pub mr: MemoryRegion,
}

unsafe extern "C" fn uart_echo_can_receive(_opaque: *mut c_void) -> c_int {
    1024
}

unsafe extern "C" fn uart_echo_receive(opaque: *mut c_void, buf: *const u8, size: c_int) {
    let s = &mut *(opaque as *mut UARTEcho);
    virtmcu_qom::chardev::qemu_chr_fe_write(&raw mut s.chr, buf, size);
}

unsafe extern "C" fn uart_echo_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    let s = &mut *(dev as *mut UARTEcho);
    virtmcu_qom::memory::memory_region_init_io(
        &raw mut s.mr,
        dev as *mut _,
        &raw const DUMMY_OPS,
        dev as *mut _,
        c"uart-echo-mmio".as_ptr(),
        0x1000,
    );
    virtmcu_qom::qdev::sysbus_init_mmio(dev as *mut _, &raw mut s.mr);

    virtmcu_qom::chardev::qemu_chr_fe_set_handlers(
        &raw mut s.chr,
        Some(uart_echo_can_receive),
        Some(uart_echo_receive),
        None,
        None,
        dev,
        core::ptr::null_mut(),
        true,
    );
}

define_properties!(UART_ECHO_PROPS, [define_prop_chr!(c"chardev".as_ptr(), UARTEcho, chr),]);

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
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(uart_echo_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

declare_device_type!(UART_ECHO_TYPE_INIT, UART_ECHO_TYPE_INFO);

/* ── Test Rust Device ────────────────────────────────────────── */

#[repr(C)]
pub struct TestRustDevice {
    pub parent: SysBusDevice,
    pub mac: MACAddr,
    pub mr: MemoryRegion,
}

unsafe extern "C" fn test_rust_realize(dev: *mut c_void, _errp: *mut *mut c_void) {
    virtmcu_qom::sim_info!("--- QOM Size Verification ---");
    virtmcu_qom::sim_info!(
        "Rust={}, C={}",
        core::mem::size_of::<virtmcu_qom::qdev::DeviceState>(),
        virtmcu_qom::virtmcu_sizeof_device_state()
    );
    virtmcu_qom::sim_info!(
        "Rust={}, C={}",
        core::mem::size_of::<virtmcu_qom::qdev::SysBusDevice>(),
        virtmcu_qom::virtmcu_sizeof_sys_bus_device()
    );
    virtmcu_qom::sim_info!(
        "Rust={}, C={}",
        core::mem::size_of::<virtmcu_qom::qdev::DeviceClass>(),
        virtmcu_qom::virtmcu_sizeof_device_class()
    );
    virtmcu_qom::sim_info!(
        "Rust={}, C={}",
        core::mem::size_of::<SSIPeripheral>(),
        virtmcu_qom::virtmcu_sizeof_ssi_peripheral()
    );
    virtmcu_qom::sim_info!(
        "Rust={}, C={}",
        core::mem::size_of::<SSIPeripheralClass>(),
        virtmcu_qom::virtmcu_sizeof_ssi_peripheral_class()
    );
    virtmcu_qom::sim_info!(
        "Rust={}, C={}",
        core::mem::size_of::<CharFrontend>(),
        virtmcu_qom::virtmcu_sizeof_char_backend()
    );
    virtmcu_qom::sim_info!("---------------------------");

    let s = &mut *(dev as *mut TestRustDevice);
    virtmcu_qom::memory::memory_region_init_io(
        &raw mut s.mr,
        dev as *mut _,
        &raw const DUMMY_OPS,
        dev as *mut _,
        c"test-rust-mmio".as_ptr(),
        0x1000,
    );
    virtmcu_qom::qdev::sysbus_init_mmio(dev as *mut _, &raw mut s.mr);
}

static TEST_PROPERTIES: [virtmcu_qom::qom::Property; 2] = [
    virtmcu_qom::define_prop_macaddr!(c"macaddr".as_ptr(), TestRustDevice, mac),
    // SAFETY: QEMU expects a zeroed Property as a sentinel at the end of the array.
    unsafe { core::mem::zeroed() },
];

unsafe extern "C" fn test_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    (*dc).realize = Some(test_rust_realize);
    virtmcu_qom::qdev::device_class_set_props_n(dc, TEST_PROPERTIES.as_ptr(), 1);
}

static TEST_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"test-rust-device".as_ptr(),
    parent: virtmcu_qom::qdev::TYPE_SYS_BUS_DEVICE,
    instance_size: core::mem::size_of::<TestRustDevice>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(test_class_init),
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

declare_device_type!(TEST_RUST_DEVICE_TYPE_INIT, TEST_TYPE_INFO);

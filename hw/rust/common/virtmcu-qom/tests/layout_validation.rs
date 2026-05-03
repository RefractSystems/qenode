#![cfg(qemu_headers_present)]
include!(concat!(env!("OUT_DIR"), "/qemu_bindings.rs"));

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::cpu::CPUState;
use virtmcu_qom::memory::{MemoryRegion, MemoryRegionOps};
use virtmcu_qom::net::{
    CanBusClientInfo, CanHostState, NetClientInfo, NetClientState, QemuCanFrame,
};
use virtmcu_qom::qdev::{DeviceClass, DeviceState, SysBusDevice};
use virtmcu_qom::qom::{ObjectClass, Property, TypeInfo};
use virtmcu_qom::sync::{QemuCond, QemuMutex};

macro_rules! assert_layout_match {
    ($our_type:ty, $qemu_type:ident) => {
        assert_eq!(
            core::mem::size_of::<$our_type>(),
            core::mem::size_of::<qemu::$qemu_type>(),
            concat!("Size mismatch for ", stringify!($our_type))
        );
        assert_eq!(
            core::mem::align_of::<$our_type>(),
            core::mem::align_of::<qemu::$qemu_type>(),
            concat!("Alignment mismatch for ", stringify!($our_type))
        );
    };
}

macro_rules! assert_offset_match {
    ($our_type:ty, $qemu_type:ident, $field:ident) => {
        assert_eq!(
            core::mem::offset_of!($our_type, $field),
            core::mem::offset_of!(qemu::$qemu_type, $field),
            concat!("Offset mismatch for ", stringify!($our_type), "::", stringify!($field))
        );
    };
}

#[test]
fn test_qom_layouts() {
    assert_layout_match!(TypeInfo, TypeInfo);
    static_assertions::assert_eq_size!(TypeInfo, qemu::TypeInfo);
    static_assertions::assert_eq_align!(TypeInfo, qemu::TypeInfo);
    assert_layout_match!(ObjectClass, ObjectClass);
    static_assertions::assert_eq_size!(ObjectClass, qemu::ObjectClass);
    static_assertions::assert_eq_align!(ObjectClass, qemu::ObjectClass);
    assert_layout_match!(Property, Property);
    static_assertions::assert_eq_size!(Property, qemu::Property);
    static_assertions::assert_eq_align!(Property, qemu::Property);

    assert_layout_match!(DeviceState, DeviceState);
    static_assertions::assert_eq_size!(DeviceState, qemu::DeviceState);
    static_assertions::assert_eq_align!(DeviceState, qemu::DeviceState);
    assert_offset_match!(DeviceState, DeviceState, id);
    assert_offset_match!(DeviceState, DeviceState, canonical_path);
    assert_offset_match!(DeviceState, DeviceState, realized);

    assert_layout_match!(DeviceClass, DeviceClass);
    static_assertions::assert_eq_size!(DeviceClass, qemu::DeviceClass);
    static_assertions::assert_eq_align!(DeviceClass, qemu::DeviceClass);
    assert_layout_match!(SysBusDevice, SysBusDevice);
    static_assertions::assert_eq_size!(SysBusDevice, qemu::SysBusDevice);
    static_assertions::assert_eq_align!(SysBusDevice, qemu::SysBusDevice);

    assert_layout_match!(MemoryRegion, MemoryRegion);
    static_assertions::assert_eq_size!(MemoryRegion, qemu::MemoryRegion);
    static_assertions::assert_eq_align!(MemoryRegion, qemu::MemoryRegion);
    assert_layout_match!(MemoryRegionOps, MemoryRegionOps);
    static_assertions::assert_eq_size!(MemoryRegionOps, qemu::MemoryRegionOps);
    static_assertions::assert_eq_align!(MemoryRegionOps, qemu::MemoryRegionOps);

    assert_layout_match!(Chardev, Chardev);
    static_assertions::assert_eq_size!(Chardev, qemu::Chardev);
    static_assertions::assert_eq_align!(Chardev, qemu::Chardev);
    assert_layout_match!(ChardevClass, ChardevClass);
    static_assertions::assert_eq_size!(ChardevClass, qemu::ChardevClass);
    static_assertions::assert_eq_align!(ChardevClass, qemu::ChardevClass);
    assert_offset_match!(ChardevClass, ChardevClass, chr_write);

    assert_layout_match!(NetClientState, NetClientState);
    static_assertions::assert_eq_size!(NetClientState, qemu::NetClientState);
    static_assertions::assert_eq_align!(NetClientState, qemu::NetClientState);
    assert_layout_match!(NetClientInfo, NetClientInfo);
    static_assertions::assert_eq_size!(NetClientInfo, qemu::NetClientInfo);
    static_assertions::assert_eq_align!(NetClientInfo, qemu::NetClientInfo);

    // assert_layout_match!(CanBusClientState, CanBusClientState);
    assert_layout_match!(CanBusClientInfo, CanBusClientInfo);
    static_assertions::assert_eq_size!(CanBusClientInfo, qemu::CanBusClientInfo);
    static_assertions::assert_eq_align!(CanBusClientInfo, qemu::CanBusClientInfo);
    assert_layout_match!(QemuCanFrame, qemu_can_frame);
    static_assertions::assert_eq_size!(QemuCanFrame, qemu::qemu_can_frame);
    static_assertions::assert_eq_align!(QemuCanFrame, qemu::qemu_can_frame);
    // assert_layout_match!(CanHostState, CanHostState);
    // static_assertions::assert_eq_size!(CanHostState, qemu::CanHostState);
    static_assertions::assert_eq_align!(CanHostState, qemu::CanHostState);

    assert_layout_match!(CPUState, CPUState);
    static_assertions::assert_eq_size!(CPUState, qemu::CPUState);
    static_assertions::assert_eq_align!(CPUState, qemu::CPUState);
    assert_offset_match!(CPUState, CPUState, cpu_index);

    assert_layout_match!(QemuMutex, QemuMutex);
    static_assertions::assert_eq_size!(QemuMutex, qemu::QemuMutex);
    static_assertions::assert_eq_align!(QemuMutex, qemu::QemuMutex);
    assert_layout_match!(QemuCond, QemuCond);
    static_assertions::assert_eq_size!(QemuCond, qemu::QemuCond);
    static_assertions::assert_eq_align!(QemuCond, qemu::QemuCond);
}

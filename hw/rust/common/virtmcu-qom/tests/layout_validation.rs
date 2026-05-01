#![cfg(qemu_headers_present)]
include!(concat!(env!("OUT_DIR"), "/qemu_bindings.rs"));

use virtmcu_qom::chardev::{Chardev, ChardevClass};
use virtmcu_qom::cpu::CPUState;
use virtmcu_qom::memory::{MemoryRegion, MemoryRegionOps};
use virtmcu_qom::net::{
    CanBusClientInfo, CanBusClientState, CanHostState, NetClientInfo, NetClientState, QemuCanFrame,
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
    assert_layout_match!(ObjectClass, ObjectClass);
    assert_layout_match!(Property, Property);

    assert_layout_match!(DeviceState, DeviceState);
    assert_offset_match!(DeviceState, DeviceState, id);
    assert_offset_match!(DeviceState, DeviceState, canonical_path);
    assert_offset_match!(DeviceState, DeviceState, realized);

    assert_layout_match!(DeviceClass, DeviceClass);
    assert_layout_match!(SysBusDevice, SysBusDevice);

    assert_layout_match!(MemoryRegion, MemoryRegion);
    assert_layout_match!(MemoryRegionOps, MemoryRegionOps);

    assert_layout_match!(Chardev, Chardev);
    assert_layout_match!(ChardevClass, ChardevClass);
    assert_offset_match!(ChardevClass, ChardevClass, chr_write);

    assert_layout_match!(NetClientState, NetClientState);
    assert_layout_match!(NetClientInfo, NetClientInfo);

    assert_layout_match!(CanBusClientState, CanBusClientState);
    assert_layout_match!(CanBusClientInfo, CanBusClientInfo);
    assert_layout_match!(QemuCanFrame, qemu_can_frame);
    assert_layout_match!(CanHostState, CanHostState);

    assert_layout_match!(CPUState, CPUState);
    assert_offset_match!(CPUState, CPUState, cpu_index);

    assert_layout_match!(QemuMutex, QemuMutex);
    assert_layout_match!(QemuCond, QemuCond);
}

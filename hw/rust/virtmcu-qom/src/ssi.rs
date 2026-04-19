use crate::qdev::{DeviceClass, DeviceState};
use core::ffi::{c_char, c_int, c_void};

pub const TYPE_SSI_PERIPHERAL: *const c_char = c"ssi-peripheral".as_ptr();

#[repr(C)]
pub struct SSIPeripheral {
    pub parent_obj: DeviceState,
    pub spc: *mut SSIPeripheralClass,
    pub cs: bool,
    pub cs_index: u8,
    pub _opaque: [u8; 168 - 152 - 8 - 2], // Pad to 168
}

#[repr(C)]
pub struct SSIPeripheralClass {
    pub parent_class: DeviceClass,
    pub realize: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, errp: *mut *mut c_void)>,
    pub transfer: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, val: u32) -> u32>,
    pub set_cs: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, select: bool) -> c_int>,
    pub cs_polarity: c_int,
    pub transfer_raw: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, val: u32) -> u32>,
}

// const _: () = assert!(core::mem::size_of::<SSIPeripheral>() == 168);
const _: () = assert!(core::mem::size_of::<SSIPeripheralClass>() == 224);

#[macro_export]
macro_rules! ssi_peripheral_class {
    ($klass:expr) => {
        unsafe {
            $crate::qom::object_class_dynamic_cast_assert(
                $klass,
                $crate::ssi::TYPE_SSI_PERIPHERAL,
                core::ptr::null(),
                0,
                core::ptr::null(),
            ) as *mut $crate::ssi::SSIPeripheralClass
        }
    };
}

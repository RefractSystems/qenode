use crate::qdev::{DeviceClass, DeviceState};
use core::ffi::{c_char, c_int, c_void};

/// A constant
pub const TYPE_SSI_PERIPHERAL: *const c_char = c"ssi-peripheral".as_ptr();

#[repr(C)]
/// A struct
pub struct SSIPeripheral {
    /// A struct field
    pub parent_obj: DeviceState, // 152
    /// A struct field
    pub spc: *mut SSIPeripheralClass, // 160
    /// A struct field
    pub cs: bool, // 161
    /// A struct field
    pub cs_index: u8, // 162
    /// A struct field
    pub _padding: [u8; 5], // 168
}

#[repr(C)]
/// A struct
pub struct SSIPeripheralClass {
    /// A struct field
    pub parent_class: DeviceClass, // 184
    /// A struct field
    pub realize: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, errp: *mut *mut c_void)>, // 192
    /// A struct field
    pub transfer: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, val: u32) -> u32>, // 200
    /// A struct field
    pub set_cs: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, select: bool) -> c_int>, // 208
    /// A struct field
    pub cs_polarity: c_int, // 212
    /// A struct field
    pub _padding: [u8; 4], // 216
    /// A struct field
    pub transfer_raw: Option<unsafe extern "C" fn(dev: *mut SSIPeripheral, val: u32) -> u32>, // 224
}

const _: () = assert!(core::mem::size_of::<SSIPeripheral>() == 168);
const _: () = assert!(core::mem::size_of::<SSIPeripheralClass>() == 224);

impl SSIPeripheral {
    /// A method
    pub fn transfer(&mut self, val: u32) -> u32 {
        unsafe {
            let klass = &*self.spc;
            if let Some(transfer_raw) = klass.transfer_raw {
                return transfer_raw(self, val);
            }
            if let Some(transfer) = klass.transfer {
                return transfer(self, val);
            }
            0
        }
    }
}

#[macro_export]
/// A macro
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

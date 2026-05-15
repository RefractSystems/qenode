use alloc::string::String;
use core::ffi::{c_char, c_int, c_void};

/// A safe wrapper for QOM string properties, encapsulating raw UTF-8 C strings.
#[repr(transparent)]
pub struct QomString(pub *mut c_char);

impl Default for QomString {
    fn default() -> Self {
        Self(core::ptr::null_mut())
    }
}

impl From<&'static core::ffi::CStr> for QomString {
    fn from(s: &'static core::ffi::CStr) -> Self {
        Self(s.as_ptr().cast_mut())
    }
}

impl QomString {
    /// Safely converts the QOM string to a Rust String.
    /// Returns an empty string if the underlying pointer is null.
    pub fn as_string(&self) -> String {
        if self.0.is_null() {
            String::new()
        } else {
            let mut len = 0;
            while unsafe { *self.0.add(len) } != 0 {
                len += 1;
            }
            let slice = unsafe { core::slice::from_raw_parts(self.0.cast::<u8>(), len) };
            // Enforce crash-only design on invalid UTF encoding (Fail Loudly mandate)
            alloc::string::String::from_utf8(slice.to_vec())
                .expect("QOM property string contains invalid UTF encoding")
        }
    }

    /// Checks if the underlying string is null.
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

// SAFETY: QomString wraps a property pointer initialized by QEMU.
unsafe impl Send for QomString {}
unsafe impl Sync for QomString {}

/// A constant
pub const LOG_UNIMP: i32 = 0x400;

use alloc::sync::Arc;
use core::marker::PhantomData;

/// A safe wrapper for a QOM Link property (Dependency Injection).
/// This replaces manual pointer management and unsafe casts.
#[repr(transparent)]
pub struct QomLink<T: ?Sized> {
    /// The underlying QOM object pointer.
    pub obj: *mut Object,
    _phantom: PhantomData<T>,
}

impl<T: ?Sized> QomLink<T> {
    /// Returns the linked object as a safe Rust trait reference.
    /// This assumes the linked object is a VirtMCU hub that exposes a pointer to the trait.
    pub fn get(&self) -> Option<Arc<T>> {
        if self.obj.is_null() {
            return None;
        }

        // Standard VirtMCU pattern: Hubs expose a 'transport_ptr' or similar.
        // Note: RFC-0023 proposes generalizing this. For Step 2, we encapsulate
        // the existing 'transport_ptr' logic used by the transport-hub.
        let mut err: *mut crate::error::Error = core::ptr::null_mut();
        let ptr_u64 =
            unsafe { object_property_get_uint(self.obj, c"transport_ptr".as_ptr(), &mut err) };

        if ptr_u64 == 0 {
            return None;
        }

        // SAFETY: We trust the Hub that it put a valid Arc<T> at this address.
        // This is still internally unsafe but encapsulated within the framework.
        let trait_ref = unsafe { &*(ptr_u64 as *const Arc<T>) };
        Some(Arc::clone(trait_ref))
    }

    /// Returns true if the link is not null.
    pub fn is_linked(&self) -> bool {
        !self.obj.is_null()
    }
}

unsafe impl<T: ?Sized> Send for QomLink<T> {}
unsafe impl<T: ?Sized> Sync for QomLink<T> {}

extern "C" {
    /// A function
    pub fn qemu_log(fmt: *const c_char, ...);
    /// A static
    pub static qemu_loglevel: c_int;
    /// A function
    pub fn type_register_static(info: *const TypeInfo) -> *mut c_void;
    /// A function
    pub fn object_class_dynamic_cast_assert(
        klass: *mut ObjectClass,
        typename: *const c_char,
        file: *const c_char,
        line: c_int,
        func: *const c_char,
    ) -> *mut ObjectClass;
    /// A function
    pub fn object_class_get_name(klass: *mut ObjectClass) -> *const c_char;
    /// A function
    pub fn register_dso_module_init(fn_: unsafe extern "C" fn(), type_: c_int);
    /// A function
    pub fn object_get_canonical_path(obj: *mut Object) -> *mut c_char;
    /// A function
    pub fn g_free(ptr: *mut c_void);
    /// A function
    pub fn object_get_root() -> *mut Object;
    /// A function
    pub fn object_dynamic_cast(obj: *mut Object, typename: *const c_char) -> *mut Object;
    /// A function
    pub fn object_child_foreach_recursive(
        obj: *mut Object,
        fn_: Option<unsafe extern "C" fn(obj: *mut Object, opaque: *mut c_void) -> c_int>,
        opaque: *mut c_void,
    ) -> c_int;
    /// A function
    pub fn object_property_add_uint64_ptr(
        obj: *mut Object,
        name: *const c_char,
        v: *const u64,
        flags: c_int,
    );
    /// A function
    pub fn object_property_add_uint32_ptr(
        obj: *mut Object,
        name: *const c_char,
        v: *const u32,
        flags: c_int,
    );
    /// A function
    /// Sets a bool property
    /// Gets a bool property
    pub fn object_property_get_bool(
        obj: *mut Object,
        name: *const c_char,
        errp: *mut *mut crate::error::Error,
    ) -> bool;
    /// Sets a bool property
    pub fn object_property_set_bool(
        obj: *mut Object,
        name: *const c_char,
        v: bool,
        errp: *mut *mut crate::error::Error,
    ) -> bool;
    /// Gets a uint property
    pub fn object_property_get_uint(
        obj: *mut Object,
        name: *const c_char,
        errp: *mut *mut crate::error::Error,
    ) -> u64;
    /// A function
    pub fn qdev_prop_allow_set_link_before_realize(
        obj: *mut Object,
        name: *const c_char,
        val: *mut Object,
        errp: *mut *mut crate::error::Error,
    );

    /// Add a link property
    pub fn object_class_property_add_link(
        klass: *mut ObjectClass,
        name: *const c_char,
        type_: *const c_char,
        offset: isize,
        check: Option<
            unsafe extern "C" fn(
                obj: *mut Object,
                name: *const c_char,
                val: *mut Object,
                errp: *mut *mut crate::error::Error,
            ),
        >,
        flags: c_int,
    );
}

/// A constant
pub const OBJ_PROP_FLAG_READ: c_int = 1;
/// Strong link flag
pub const OBJ_PROP_LINK_STRONG: c_int = 1;
/// A constant
pub const OBJ_PROP_FLAG_WRITE: c_int = 2;
/// A constant
pub const OBJ_PROP_FLAG_READWRITE: c_int = OBJ_PROP_FLAG_READ | OBJ_PROP_FLAG_WRITE;

/// A constant
pub const TYPE_DEVICE: *const c_char = c"device".as_ptr();
/// A constant
pub const MODULE_INIT_QOM: c_int = 3;

#[macro_export]
/// A macro
macro_rules! qemu_log_mask {
    ($mask:expr, $($arg:tt)*) => {{
        unsafe {
            if ($crate::qom::qemu_loglevel & $mask) != 0 {
                $crate::sim_info!($($arg)*);
            }
        }
    }};
}

#[macro_export]
/// A macro
macro_rules! device_class {
    ($klass:expr) => {
        unsafe {
            $crate::qom::object_class_dynamic_cast_assert(
                $klass,
                $crate::qdev::TYPE_DEVICE,
                core::ptr::null(),
                0,
                core::ptr::null(),
            ) as *mut $crate::qdev::DeviceClass
        }
    };
}

#[repr(C)]
/// A struct
pub struct Object {
    /// A struct field
    pub class: *mut ObjectClass,
    /// A struct field
    pub free: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub properties: *mut c_void,
    /// A struct field
    pub ref_: c_int,
    /// A struct field
    pub parent: *mut Object,
}

#[repr(C)]
/// A struct
pub struct ObjectClass {
    /// A struct field
    pub type_: *mut c_void,
    /// A struct field
    pub interfaces: *mut c_void,
    /// A struct field
    pub object_cast_cache: [*mut c_char; 4],
    /// A struct field
    pub class_cast_cache: [*mut c_char; 4],
    /// A struct field
    pub unparent: *mut c_void,
    /// A struct field
    pub properties: *mut c_void,
}

const _: () = assert!(core::mem::size_of::<ObjectClass>() == 96);

#[repr(C)]
/// A struct
pub struct TypeInfo {
    /// A struct field
    pub name: *const c_char,
    /// A struct field
    pub parent: *const c_char,
    /// A struct field
    pub instance_size: usize,
    /// A struct field
    pub instance_align: usize,
    /// A struct field
    pub instance_init: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub instance_post_init: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub instance_finalize: Option<unsafe extern "C" fn(obj: *mut Object)>,
    /// A struct field
    pub abstract_: bool,
    /// A struct field
    pub class_size: usize,
    /// A struct field
    pub class_init: Option<unsafe extern "C" fn(klass: *mut ObjectClass, data: *const c_void)>,
    /// A struct field
    pub class_base_init: Option<unsafe extern "C" fn(klass: *mut ObjectClass, data: *const c_void)>,
    /// A struct field
    pub class_data: *const c_void,
    /// A struct field
    pub interfaces: *const c_void,
}

#[repr(C)]
/// A struct
pub struct Property {
    /// A struct field
    pub name: *const c_char,
    /// A struct field
    pub info: *const c_void,
    /// A struct field
    pub offset: isize,
    /// A struct field
    pub link_type: *const c_char,
    /// A struct field
    pub bitmask: u64,
    /// A struct field
    pub defval: u64,
    /// A struct field
    pub arrayinfo: *const c_void,
    /// A struct field
    pub arrayoffset: c_int,
    /// A struct field
    pub arrayfieldsize: c_int,
    /// A struct field
    pub bitnr: u8,
    /// A struct field
    pub set_default: bool,
    /// A struct field
    pub _padding: [u8; 6],
}

const _: () = assert!(core::mem::size_of::<Property>() == 72);

// SAFETY: TypeInfo contains function pointers and static metadata for QOM types.
// It is used as a static registration struct.
unsafe impl Sync for TypeInfo {}
// SAFETY: Property contains static metadata for device properties.
unsafe impl Sync for Property {}
// SAFETY: Property contains static metadata for device properties.
unsafe impl Send for Property {}

#[macro_export]
/// A macro
macro_rules! declare_device_type {
    ($init_fn:ident, $type_info:expr) => {
        #[used]
        #[no_mangle]
        #[cfg_attr(target_os = "linux", link_section = ".init_array")]
        #[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
        #[cfg_attr(target_os = "windows", link_section = ".CRT$XCU")]
        pub static $init_fn: extern "C" fn() = {
            extern "C" fn wrapper() {
                #[cfg(not(miri))]
                // SAFETY: register_dso_module_init is a QEMU-provided function to register
                // a module initialization function. It is safe to call during global
                // constructors as long as the parameters are valid.
                unsafe {
                    if option_env!("VIRTMCU_UNIT_TEST").is_none() {
                        $crate::qom::register_dso_module_init(
                            real_init,
                            $crate::qom::MODULE_INIT_QOM,
                        );
                    }
                }
            }
            unsafe extern "C" fn real_init() {
                $crate::qom::type_register_static(&$type_info);
            }
            wrapper
        };
    };
}

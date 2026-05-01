use core::ffi::{c_char, c_void};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::qom::TypeInfo;

// A simple static to track if our mock was called
static mut MOCK_CALLED: bool = false;

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn register_dso_module_init(fn_: unsafe extern "C" fn(), _type_: core::ffi::c_int) {
    // Mock for testing - we need to execute the function to test type_register_static
    // SAFETY: test only
    unsafe {
        fn_();
    }
}

#[cfg(not(miri))]
#[no_mangle]
pub static qdev_prop_uint32: u64 = 0;
#[cfg(not(miri))]
#[no_mangle]
pub static qdev_prop_uint64: u64 = 0;
#[cfg(not(miri))]
#[no_mangle]
pub static qdev_prop_bool: u64 = 0;
#[cfg(not(miri))]
#[no_mangle]
pub static qdev_prop_string: u64 = 0;
#[cfg(not(miri))]
#[no_mangle]
pub static qdev_prop_macaddr: u64 = 0;

#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn type_register_static(info: *const TypeInfo) -> *mut c_void {
    // SAFETY: test only
    unsafe {
        MOCK_CALLED = true;
        // Verify some fields
        let name_str =
            core::ffi::CStr::from_ptr((*info).name as *const c_char).to_str().unwrap_or_default();
        assert_eq!(name_str, "test-device");

        let parent_str =
            core::ffi::CStr::from_ptr((*info).parent as *const c_char).to_str().unwrap_or_default();
        assert_eq!(parent_str, "sys-bus-device");

        assert_eq!((*info).instance_size, 128);
    }
    core::ptr::null_mut()
}

static TEST_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"test-device".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: 128,
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: None,
    abstract_: false,
    class_size: 0,
    class_init: None,
    class_base_init: None,
    class_data: core::ptr::null(),
    interfaces: core::ptr::null(),
};

// Use the macro to generate the init function
declare_device_type!(dso_test_init, TEST_TYPE_INFO);

#[test]
fn test_declare_device_type_macro() {
    #[cfg(not(miri))]
    // SAFETY: test only
    unsafe {
        MOCK_CALLED = false;
        dso_test_init();
        assert!(MOCK_CALLED, "The init function did not call type_register_static");
    }
}

#[test]
fn test_macaddr_property_init() {
    use virtmcu_qom::qdev::MACAddr;
    struct TestDev {
        mac: MACAddr,
    }
    let p = virtmcu_qom::define_prop_macaddr!(c"test-mac".as_ptr(), TestDev, mac);
    assert!(!p.name.is_null());
    assert_eq!(p.offset, core::mem::offset_of!(TestDev, mac) as isize);
    assert!(!p.info.is_null());
    assert!(!p.set_default);
}

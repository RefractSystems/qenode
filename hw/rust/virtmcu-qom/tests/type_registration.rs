use core::ffi::{c_char, c_void};
use virtmcu_qom::declare_device_type;
use virtmcu_qom::qom::TypeInfo;

// A simple static to track if our mock was called
static mut MOCK_CALLED: bool = false;

// Provide the mock implementation of the extern "C" function that QEMU usually provides.
#[no_mangle]
pub extern "C" fn type_register_static(info: *const TypeInfo) -> *mut c_void {
    unsafe {
        MOCK_CALLED = true;
        // Verify some fields
        let name_str = std::ffi::CStr::from_ptr((*info).name as *const core::ffi::c_char)
            .to_str()
            .unwrap();
        assert_eq!(name_str, "test-device");

        let parent_str = std::ffi::CStr::from_ptr((*info).parent as *const core::ffi::c_char)
            .to_str()
            .unwrap();
        assert_eq!(parent_str, "sys-bus-device");

        assert_eq!((*info).instance_size, 128);
    }
    std::ptr::null_mut()
}

static TEST_TYPE_INFO: TypeInfo = TypeInfo {
    name: b"test-device\0".as_ptr() as *const c_char,
    parent: b"sys-bus-device\0".as_ptr() as *const c_char,
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
    unsafe {
        MOCK_CALLED = false;
        dso_test_init();
        assert!(
            MOCK_CALLED,
            "The init function did not call type_register_static"
        );
    }
}

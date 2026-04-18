use virtmcu_qom::qom::TypeInfo;
use virtmcu_qom::declare_device_type;

static TEST_TYPE_INFO: TypeInfo = TypeInfo {
    name: b"test-rust-device\0".as_ptr() as *const core::ffi::c_char,
    parent: b"sys-bus-device\0".as_ptr() as *const core::ffi::c_char,
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

declare_device_type!(dso_test_init, TEST_TYPE_INFO);

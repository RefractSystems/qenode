use virtmcu_qom::declare_device_type;
use virtmcu_qom::qom::TypeInfo;

static TEST_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"test-rust-device".as_ptr(),
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

declare_device_type!(dso_test_init, TEST_TYPE_INFO);

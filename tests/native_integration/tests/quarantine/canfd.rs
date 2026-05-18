use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_canfd_plugin_loads() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Verify CAN plugin instantiates successfully (loads without crashing)
    let _env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb")
                .add_qemu_arg("-object")
                .add_qemu_arg("can-bus,id=canbus0")
                .add_qemu_arg("-object")
                .add_qemu_arg("can-host-virtmcu,id=canhost0,canbus=canbus0,node=0,router={ROUTER_ENDPOINT},topic=sim/can")
        )
        .with_timeout(10)
        .build()
        .await?;

    Ok(())
}

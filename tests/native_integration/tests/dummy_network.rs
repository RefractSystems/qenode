use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dummy_ping_pong_network() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/dummy_ping_pong/pinger.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb")
                .add_qemu_arg("-device")
                .add_qemu_arg("rust-dummy,base-addr=0x09005000,node-id=0,topic=dummy_bus"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/dummy_ping_pong/ponger.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb")
                .add_qemu_arg("-device")
                .add_qemu_arg("rust-dummy,base-addr=0x09005000,node-id=1,topic=dummy_bus"),
        )
        .with_timeout(30)
        .run_test(|env| {
            Box::pin(async move {
                env.wait_for_output(0, "Node 0: Pinger starting").await?;
                env.wait_for_output(1, "Node 1: Ponger starting").await?;

                env.step_clock(50_000_000, 10_000_000).await?;

                env.wait_for_output(1, "Node 1: Ping received!").await?;
                env.wait_for_output(0, "Node 0: Pong received! Test complete.")
                    .await?;
                env.wait_for_output(1, "Node 1: Pong sent! Test complete.")
                    .await?;

                Ok(())
            })
        })
        .await
}

use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_coordinator_stress() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Spawn 3 orchestrated nodes to ensure Zenoh PDES coordinator works smoothly under load.
    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .add_node(
            NodeConfig::new(2)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(30)
        .build()
        .await?;

    let iters = if std::env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1" {
        50
    } else {
        200
    };
    env.step_clock(iters * 1_000_000, 1_000_000).await?;

    env.wait_for_output(0, "HI").await?;
    env.wait_for_output(1, "HI").await?;
    env.wait_for_output(2, "HI").await?;

    Ok(())
}

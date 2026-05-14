use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, TelemetryMonitor, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_coordinator_topology() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/telemetry_wfi/test_wfi.elf")
                .with_yaml_path("tests/fixtures/guest_apps/telemetry_wfi/test_telemetry.yaml")
                .add_qemu_arg("-device")
                .add_qemu_arg("telemetry,transport=hub0"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/telemetry_wfi/test_wfi.elf")
                .with_yaml_path("tests/fixtures/guest_apps/telemetry_wfi/test_telemetry.yaml")
                .add_qemu_arg("-device")
                .add_qemu_arg("telemetry,transport=hub0"),
        )
        .with_timeout(10)
        .build()
        .await?;

    let _monitor0 = TelemetryMonitor::new(&env.session(), 0).await?;
    let _monitor1 = TelemetryMonitor::new(&env.session(), 1).await?;

    // Step clock for both nodes
    env.step_clock(10_000_000, 1_000_000).await?;

    Ok(())
}

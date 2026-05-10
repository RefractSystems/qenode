use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, TelemetryMonitor, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_halt_hook_multiplexing() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Test that when both `telemetry` and `clock` (injected automatically)
    // are loaded, they both receive CPU halt hooks.
    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/telemetry_wfi/test_wfi.elf")
                .with_dtb_path("tests/fixtures/guest_apps/telemetry_wfi/test_telemetry.dtb")
                .add_qemu_arg("-device")
                .add_qemu_arg("telemetry,transport=virtmcu-transport-hub"),
        )
        .with_timeout(10)
        .build()
        .await?;

    let monitor = TelemetryMonitor::new(&env.session(), 0).await?;

    // Advance virtual time.
    // If the clock plugin didn't get its hook, step_clock will hang/timeout.
    // If the telemetry plugin didn't get its hook, the monitor wait will timeout.
    env.step_clock(100_000_000, 10_000_000).await?;

    let traces = monitor.wait_for_traces(1, 5).await?;
    assert!(
        !traces.is_empty(),
        "Telemetry plugin did not receive CPU halt events!"
    );

    Ok(())
}

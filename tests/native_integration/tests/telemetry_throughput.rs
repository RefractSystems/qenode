use anyhow::Result;
use tokio::time::Instant;
use tracing::info;
use virtmcu_test_runner::{NodeConfig, TelemetryMonitor, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_telemetry_throughput() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Use the existing IRQ storm ASM and yaml from tests/fixtures
    let env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/telemetry_wfi/test_irq_storm.elf")
                .with_dtb_path("tests/fixtures/guest_apps/telemetry_wfi/test_telemetry.dtb")
                .add_qemu_arg("-device")
                .add_qemu_arg("telemetry,transport=hub0")
                .orchestrated(false),
        )
        .with_timeout(20)
        .build()
        .await?;

    env.run_test(|env| {
        Box::pin(async move {
            let monitor = TelemetryMonitor::new(&env.session(), 0).await?;

            // Send something to UART to trigger IRQ storm (PL011 RX IRQ)
            let uart_rx_topic = "sim/uart/0/rx";
            env.session()
                .put(uart_rx_topic, "trigger_irq\n")
                .await
                .map_err(|e| anyhow::anyhow!("Zenoh error: {}", e))?;

            let start_time = Instant::now();

            // Since orchestrated(false), wait for traces using wall-clock time
            // Wait for at least 1000 to prove high throughput.
            let traces = monitor.wait_for_traces(1000, 30).await;

            let elapsed = start_time.elapsed();

            assert!(
                traces.is_ok(),
                "Failed to receive 1000 telemetry events within timeout"
            );

            let trace_count = traces.unwrap().len();
            info!("Received {} traces in {:?}", trace_count, elapsed);
            assert!(trace_count >= 1000, "Throughput too low");

            Ok(())
        })
    })
    .await;

    Ok(())
}

use anyhow::Result;
use virtmcu_test_runner::{monitors::SpiEchoMonitor, NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_spi_echo_baremetal() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/spi_bridge/spi_echo.elf")
                .with_yaml_path("tests/fixtures/guest_apps/spi_bridge/spi_test.yaml")
                // Add the router endpoint as a global property so the YAML-generated SPI bridge connects
                .add_qemu_arg("-global")
                .add_qemu_arg("virtmcu-spi-bridge.router={ROUTER_ENDPOINT}")
                .orchestrated(false),
        )
        .with_timeout(10)
        .build()
        .await?;

    // The fixture firmware `spi_echo.elf` communicates on spi0 with CS=0
    let _spi_monitor = SpiEchoMonitor::new(&env.session(), "spi0", 0).await?;

    // Unfreeze and advance virtual time natively.
    // The firmware performs the loopback and writes 'P' to UART on success, or 'F' on failure.
    env.step_clock(100_000_000, 1_000_000).await?;

    env.wait_for_output(0, "P").await?;

    Ok(())
}

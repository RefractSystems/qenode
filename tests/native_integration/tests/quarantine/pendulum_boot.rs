use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_pendulum_boot_smoke() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/pendulum_controller/controller.elf")
                .with_yaml_path("tests/fixtures/guest_apps/pendulum_controller/board.yaml")
                .orchestrated(false),
        )
        .with_timeout(10)
        .run_test(|env| {
            Box::pin(async move {
                // Gate 1: firmware boots and UART works
                env.wait_for_output(0, "Pendulum PID Controller Starting...")
                    .await?;
                Ok(())
            })
        })
        .await
}

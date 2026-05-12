use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_yaml_platform_boot() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_yaml_path("tests/fixtures/guest_apps/yaml_boot/test_board.yaml")
                .orchestrated(false),
        )
        .with_timeout(10)
        .run_test(|env| {
            Box::pin(async move {
                env.wait_for_output(0, "HI").await?;
                Ok(())
            })
        })
        .await
}

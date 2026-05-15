use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

async fn run_ping_pong_test(transport: &str) -> Result<()> {
    VirtmcuTestEnv::builder()
        .with_transport_override(transport)
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/reference_ping_pong/pinger.elf")
                .with_yaml_path("worlds/reference_ping_pong.yml"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/reference_ping_pong/ponger.elf")
                .with_yaml_path("worlds/reference_ping_pong.yml"),
        )
        .with_timeout(30)
        .run_test(|env| {
            Box::pin(async move {
                env.wait_for_output(0, "Node 0: Pinger starting").await?;
                env.wait_for_output(0, "Node 0: Ping sent, waiting for Pong...")
                    .await?;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_reference_ping_pong_zenoh() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    run_ping_pong_test("zenoh").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_reference_ping_pong_unix() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    run_ping_pong_test("unix").await
}

use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_bql_starvation_avoided() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/bql_stress/main.elf")
                .with_yaml_path("tests/fixtures/guest_apps/bql_stress/board.yaml")
                .orchestrated(true),
        )
        .with_timeout(10)
        .build()
        .await?;

    // Wait for the firmware to start and enter the tight loop
    env.wait_for_output(0, "BQL stress starting").await?;
    env.wait_for_output(0, "Tight polling loop").await?;

    // Allow the guest to spin for a full 100 milliseconds of virtual time.
    // If the BQL is locked up, the QEMU main loop will freeze here and hit the with_timeout(10) bound.
    env.step_clock(100_000_000, 10_000_000).await?;

    // Construct a basic dummy packet to signal the peripheral to wake up
    // The rust-dummy peripheral doesn't strictly parse a complex packet yet,
    // but sending to its transport topic should trigger its internal callbacks.
    let dummy_topic = "sim/dummy/rx/0";
    let payload = virtmcu_api::encode_frame(env.vtime() + 10_000_000, 0, &[1, 2, 3, 4]);

    env.session()
        .put(dummy_topic, payload)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to publish to dummy: {}", e))?;

    // Advance the clock again. If BQL yielding works, the QEMU main loop
    // grabs the lock, processes the packet, and updates REG_DUMMY_STATUS.
    env.step_clock(50_000_000, 10_000_000).await?;

    // Verify the guest broke out of the loop
    env.wait_for_output(0, "Starvation avoided").await?;

    Ok(())
}

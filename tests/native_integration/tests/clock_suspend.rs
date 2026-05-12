use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_clock_suspend() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Verify clock properly suspends. If we don't step the clock, QEMU shouldn't print "HI"
    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(5)
        .build()
        .await?;

    // We do NOT step the clock.
    let res = env.wait_for_output_passive(0, "HI").await;
    assert!(
        res.is_err(),
        "Clock should be suspended, but QEMU advanced!"
    );

    Ok(())
}

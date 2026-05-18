use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[should_panic(expected = "Unregistered packet received!")]
async fn test_reference_ping_pong_bad_topology() {
    let _ = tracing_subscriber::fmt::try_init();

    let result = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/reference_ping_pong/pinger.elf")
                .with_yaml_path("worlds/reference_ping_pong_bad.yml"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/reference_ping_pong/ponger.elf")
                .with_yaml_path("worlds/reference_ping_pong_bad.yml"),
        )
        .with_timeout(5)
        .run_test(|env| {
            Box::pin(async move {
                env.wait_for_output(0, "Node 0: Pinger starting").await?;
                env.wait_for_output(1, "Node 1: Ponger starting").await?;

                env.step_clock(50_000_000, 10_000_000).await?;

                Ok(())
            })
        })
        .await;

    // Check if we got an error, and if the error string contains the panic message
    if let Err(e) = result {
        let err_str = e.to_string();
        if err_str.contains("Unregistered packet received!") || err_str.contains("panic") {
            panic!("Unregistered packet received!");
        } else {
            panic!("Failed with other error: {}", err_str);
        }
    } else {
        panic!("Test succeeded but was expected to fail!");
    }
}

use anyhow::Result;
use virtmcu_test_runner::{monitors::ActuatorMonitor, NodeConfig, VirtmcuTestEnv};
use virtmcu_wire::topics::sim_topic;

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_actuator_zenoh_publish() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // The actuator tests replace `router: "ZENOH_ROUTER_ENDPOINT"` with `transport: virtmcu-transport-hub`
    // because `actuator` device is now bound to the `virtmcu-transport-hub` instead of its own session.
    let yaml_path = "tests/fixtures/guest_apps/actuator/board.yaml";

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/actuator/actuator.elf")
                .with_yaml_path(yaml_path)
                .orchestrated(true),
        )
        .with_timeout(60)
        .run_test(|env| {
            Box::pin(async move {
                let topic_42 = sim_topic::actuator_control("0", 42);
                let topic_99 = sim_topic::actuator_control("0", 99);
                let topics = vec![topic_42.as_str(), topic_99.as_str()];
                let monitor = ActuatorMonitor::new(&env.session(), &topics).await?;

                env.wait_for_output(0, "Actuator test firmware starting")
                    .await?;
                env.step_clock(500_000_000, 10_000_000).await?;
                env.wait_for_output(0, "Test complete").await?;

                // The actuator guest app performs multiple math operations and writes them
                let found = monitor
                    .wait_for_responses(30, |msgs| {
                        let mut success_1 = false;
                        let mut success_2 = false;

                        for (topic, _vtime, vals) in msgs {
                            if topic == &topic_42 && (vals[0] - std::f64::consts::PI).abs() < 0.001
                            {
                                success_1 = true;
                            } else if topic == &topic_99
                                && vals.len() == 3
                                && vals == &[1.0, 2.0, 3.0]
                            {
                                success_2 = true;
                            }
                        }
                        success_1 && success_2
                    })
                    .await?;

                assert!(found, "Did not receive all control signals");

                Ok(())
            })
        })
        .await
}

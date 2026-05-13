use anyhow::Result;
use std::process::Command;
use virtmcu_api::topics::sim_topic;
use virtmcu_test_runner::{monitors::ActuatorMonitor, NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_pendulum_closed_loop() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let firmware_path = "tests/fixtures/guest_apps/pendulum_controller/controller.elf";
    let yaml_path = "tests/fixtures/guest_apps/pendulum_controller/board.yaml";
    let resd_path = "tests/fixtures/guest_apps/pendulum_controller/pendulum_angles.resd";

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path(firmware_path)
                .with_yaml_path(yaml_path)
                .orchestrated(true),
        )
        .with_timeout(30)
        .build()
        .await?;

    let session = env.session();
    let topic_wildcard = sim_topic::actuator_control_wildcard("0");
    let monitor = ActuatorMonitor::new(&session, &[&topic_wildcard]).await?;

    // Spawn resd_replay as a separate process
    let resd_replay_bin = env.find_binary("virtmcu-resd-replay")?;

    let mut resd_cmd = Command::new(resd_replay_bin);
    resd_cmd
        .arg(resd_path)
        .arg("0") // node_id
        .arg("1000000"); // delta_ns (1ms)

    // Set ZENOH_CONNECT if needed (VirtmcuTestEnv sets up a local router usually)
    // Actually, VirtmcuTestEnv handles the router, so we should connect to it.
    if let Some(router_endpoint) = env.router_endpoint() {
        resd_cmd.env("ZENOH_CONNECT", router_endpoint);
    }

    let mut resd_handle = resd_cmd.spawn()?;

    // Step the clock for 20 quanta of 1ms each
    for _ in 0..20 {
        env.step_clock(1_000_000, 1_000_000).await?;
    }

    // Wait for resd_replay to finish
    let _ = resd_handle.wait()?;

    // Assert that the ActuatorMonitor received at least 15 actuator commands
    let found = monitor
        .wait_for_responses(5, |msgs| {
            let mut count = 0;
            for (_topic, _vtime, vals) in msgs {
                if vals.len() == 1 {
                    let val = vals[0];
                    if (-500.0..=500.0).contains(&val) {
                        count += 1;
                    }
                }
            }
            count >= 15
        })
        .await?;

    assert!(found, "Did not receive enough valid actuator commands");

    // Assert that UART output contains "Angle:"
    env.wait_for_output(0, "Angle:").await?;

    Ok(())
}

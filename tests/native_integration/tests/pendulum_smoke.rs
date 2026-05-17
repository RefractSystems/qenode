use anyhow::Result;
use virtmcu_test_runner::{monitors::ActuatorMonitor, NodeConfig, VirtmcuTestEnv};
use virtmcu_wire::encode_frame;
use virtmcu_wire::topics::sim_topic;

// Validates the full peripheral stack without physics:
// boot → UART → clock sync → sensor inject → actuator publish.
// This must pass before pendulum_loop (Level 3) is even attempted.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_pendulum_peripherals_smoke() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/pendulum_controller/controller.elf")
                .with_yaml_path("tests/fixtures/guest_apps/pendulum_controller/board.yaml")
                .orchestrated(true),
        )
        .with_timeout(20)
        .build()
        .await?;

    // Gate 1: firmware boots and UART works
    env.wait_for_output(0, "Pendulum PID Controller Starting...")
        .await?;
    env.wait_for_output(0, "Entering main loop...").await?;
    env.wait_for_output(0, "Calling read_sensor()...").await?;

    // Gate 2: clock sync handshake completes (step_clock would hang if broken)
    env.step_clock(10_000_000, 10_000_000).await?;

    // Gate 3 & 4: inject sensor readings until actuator command is received
    let topic = sim_topic::sensor_data("0", 0);
    let actuator_topic = sim_topic::actuator_control("0", 0);
    let monitor = ActuatorMonitor::new(&env.session(), &[&actuator_topic]).await?;

    let mut found = false;
    for _ in 0..200 {
        let inject_vtime = env.vtime();
        let payload = encode_frame(inject_vtime, 0, &0.5_f64.to_le_bytes());
        env.session()
            .put(&topic, payload)
            .await
            .map_err(|e| anyhow::anyhow!("Zenoh error: {e}"))?;

        env.step_clock(10_000_000, 10_000_000).await?;

        let msgs = monitor.captured_messages.lock().unwrap();
        if !msgs.is_empty() {
            found = true;
            break;
        }
    }

    if !found {
        let uart = env.uart_buffer(0).await;
        println!("UART Output:\n{}", uart);
    }
    assert!(
        found,
        "Actuator peripheral produced no output after multiple injections"
    );

    // Gate 5: UART printed sensor data it received
    env.wait_for_output(0, "Angle:").await?;

    Ok(())
}

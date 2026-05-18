use anyhow::Result;
use cyber_bridge::resd_parser::ResdParser;
use std::path::PathBuf;
use virtmcu_test_runner::{monitors::ActuatorMonitor, NodeConfig, VirtmcuTestEnv};
use virtmcu_wire::topics::sim_topic;

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_pendulum_closed_loop() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Find workspace root
    let mut workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    workspace_root.pop(); // tests/
    workspace_root.pop(); // workspace root

    let guest_app_dir = workspace_root.join("tests/fixtures/guest_apps/pendulum_controller");
    let firmware_path = guest_app_dir.join("controller.elf");
    let yaml_path = guest_app_dir.join("board.yaml");
    let resd_path = guest_app_dir.join("pendulum_angles.resd");
    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path(firmware_path.to_str().unwrap())
                .with_yaml_path(yaml_path.to_str().unwrap())
                .orchestrated(true),
        )
        .with_timeout(60)
        .build()
        .await?;
    // Wait for boot
    env.wait_for_output(0, "Pendulum PID Controller Starting...")
        .await?;
    env.wait_for_output(0, "Entering main loop...").await?;
    env.wait_for_output(0, "Calling read_sensor()...").await?;

    let session = env.session();
    let actuator_topic = sim_topic::actuator_control("0", 0);
    let monitor = ActuatorMonitor::new(&session, &[&actuator_topic]).await?;

    // Load RESD data
    let mut parser = ResdParser::new(&resd_path);
    assert!(parser.init(), "Failed to parse RESD file");
    let sensors = parser.sensors;

    let mut current_vtime_ns = env.vtime();
    let step_ns = 10_000_000; // 10ms steps for faster simulation

    let mut count = 0;

    // Step clock and inject data
    for _i in 0..500 {
        // Step total 1000ms
        for ((_sample_type, channel_id), sensor) in &sensors {
            let topic = sim_topic::sensor_data("0", *channel_id as u32);
            println!("Publishing to topic: {}", topic);
            let vals = sensor.get_reading(current_vtime_ns);
            let mut data_payload = Vec::with_capacity(vals.len() * 8);
            for v in vals {
                data_payload.extend_from_slice(&v.to_le_bytes());
            }
            let payload = virtmcu_wire::encode_frame(current_vtime_ns, 0, &data_payload);
            session
                .put(&topic, payload)
                .await
                .map_err(|e| anyhow::anyhow!("Zenoh error: {e}"))?;
        }

        env.step_clock(step_ns, step_ns).await?;
        current_vtime_ns += step_ns;

        // Drain UART to see what's happening
        let _ = env.wait_for_output_passive(0, "").await;

        // Check if we got enough commands yet
        let msgs = monitor.captured_messages.lock().unwrap();
        count = 0;
        for (topic, _vtime, vals) in msgs.iter() {
            if topic == &actuator_topic && vals.len() == 1 && (-500.0..=500.0).contains(&vals[0]) {
                count += 1;
            }
        }
        if count >= 3 {
            break;
        }
    }

    if count < 3 {
        let uart = env.uart_buffer(0).await;
        println!("UART Output:\n{}", uart);
    }
    assert!(count >= 3, "Received only {} actuator commands", count);

    // Assert that UART output contains "Angle:"
    env.wait_for_output(0, "Angle:").await?;

    Ok(())
}

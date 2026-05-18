#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"

use anyhow::{Context, Result};
use tokio::process::Command;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_pendulum_compose_e2e() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // 1. Setup Environment (cyber-node)
    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/pendulum_controller/controller.elf")
                .with_yaml_path("tests/fixtures/guest_apps/pendulum_controller/board.yaml")
                .orchestrated(true),
        )
        .with_federation_id("test_e2e")
        .with_timeout(30)
        .build()
        .await
        .context("Failed to build test environment")?;

    let endpoint = env.router_endpoint().expect("Router endpoint must be set");

    // 2. Spawn physical node
    let mut phys_cmd = Command::new(
        env.find_binary("virtmcu-physical-node")
            .context("virtmcu-physical-node not found")?,
    );
    phys_cmd
        .arg("--federation-id")
        .arg("test_e2e")
        .arg("--transport")
        .arg("zenoh")
        .arg("--connect")
        .arg(&endpoint)
        .arg("--node-id")
        .arg("0")
        .arg("--delta-ns")
        .arg("1000000") // 1ms
        .arg("--timeout-ms")
        .arg("30000")
        .arg("--plant")
        .arg("embedded")
        .arg("--n-sensors")
        .arg("1")
        .arg("--n-actuators")
        .arg("1");

    let phys_child = phys_cmd
        .spawn()
        .context("Failed to spawn virtmcu-physical-node")?;
    env.register_child(phys_child);

    // 3. Spawn mock physics engine
    let mut mock_cmd = Command::new(
        env.find_binary("pendulum-mock-physics")
            .context("pendulum-mock-physics not found")?,
    );
    mock_cmd
        .arg("--node-id")
        .arg("0")
        .arg("--delta-ns")
        .arg("1000000"); // 1ms

    // Override ZENOH_CONNECT for mock-physics since it doesn't take --connect arg
    mock_cmd.env("ZENOH_CONNECT", &endpoint);

    let mock_child = mock_cmd
        .spawn()
        .context("Failed to spawn pendulum-mock-physics")?;
    env.register_child(mock_child);

    // 4. Type-safe Assertions
    env.wait_for_output(0, "Pendulum PID Controller Starting...")
        .await
        .context("Firmware did not start correctly")?;

    // Step clock: advance by 10 quanta (each is 10ms for this test setup historically, but let's step exactly what's needed)
    // Actually, wait_for_output already steps the clock internally if orchestrated.
    // Let's just wait for "Angle:", which confirms the closed loop ran.
    env.wait_for_output(0, "Angle:")
        .await
        .context("Did not see 'Angle:' output from firmware")?;

    // Teardown is guaranteed by VirtmcuTestEnv's Drop / run_test pattern.
    env.teardown().await;

    Ok(())
}

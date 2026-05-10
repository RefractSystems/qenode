use anyhow::Result;
use tokio::time::Duration;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_shutdown_while_blocked() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Spawns a simulation, starts it, then immediately stops it to catch teardown races.
    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(10)
        .build()
        .await?;

    // Give it a few ms to boot deterministically
    env.step_clock(5_000_000, 1_000_000).await?;

    // Simulation context exit should be clean when it drops.
    // The drop handler will send the QMP quit signal and clean up gracefully.
    drop(env);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_shutdown_during_vta_step() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // Stress the teardown by dropping the environment while a VTA step is in flight.
    let env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(10)
        .build()
        .await?;

    // We start stepping in the background so we can drop the env
    // We clone the Arc-wrapped session directly to simulate concurrent traffic
    // or just drop the env. Wait, we can't `tokio::spawn` with `&mut env`.
    // Instead we can just trigger a massive step and then drop the env immediately.

    // Send a massive step request using the low-level session
    let advance_topic = "sim/clock/advance/0";
    let mut payload = Vec::with_capacity(24);
    let step_ns: u64 = 10_000_000_000;
    payload.extend_from_slice(&step_ns.to_le_bytes());
    payload.extend_from_slice(&step_ns.to_le_bytes()); // target
    payload.extend_from_slice(&1u64.to_le_bytes()); // quantum

    let session = env.session();
    let replies = session.get(advance_topic).payload(payload).await.unwrap();

    // Give it a tiny bit to actually start processing the request
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Shutdown while step is in flight
    drop(env);

    // Collect replies. QEMU might have died, so this could fail or hang if not handled.
    // The test runner infrastructure should clean up QEMU resulting in socket drop and zenoh exit.
    while let Ok(reply) = replies.recv_async().await {
        let _ = reply.result();
    }

    Ok(())
}

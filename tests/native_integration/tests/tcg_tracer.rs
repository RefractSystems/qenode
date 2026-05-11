use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use virtmcu_api::telemetry_generated::virtmcu::telemetry::TraceEvent;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_tcg_tracer_loads_and_streams() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // In a cargo workspace, CARGO_MANIFEST_DIR is tests/native_integration
    let mut plugin_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    plugin_path.pop(); // Up to tests/
    plugin_path.pop(); // Up to workspace root
    plugin_path.push("target/debug/libtcg_tracer.so");

    let plugin_arg = format!("{},node_id=0,transport=zenoh", plugin_path.display());

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/telemetry_wfi/test_wfi.elf")
                // Use a simple, known-good topology to avoid complex boot loops
                .with_dtb_path("tests/fixtures/guest_apps/telemetry_wfi/test_telemetry.dtb")
                .add_qemu_arg("-plugin")
                .add_qemu_arg(&plugin_arg)
                .orchestrated(true),
        )
        .with_timeout(10)
        .build()
        .await?;

    let session = env.session();
    let subscriber = session
        .declare_subscriber("sim/telemetry/trace/0/insn")
        .await
        .expect("Failed to subscribe");

    // Advance clock to trigger execution
    env.step_clock(10_000_000, 10_000_000).await?;

    // We should see a flood of instructions
    let mut got_insn = false;
    // Wait for at least one valid instruction trace
    let start = tokio::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if let Ok(msg) =
            tokio::time::timeout(Duration::from_millis(100), subscriber.recv_async()).await
        {
            if let Ok(msg) = msg {
                let payload = msg.payload().to_bytes();
                let event = flatbuffers::root::<TraceEvent>(&payload).expect("Invalid FB");
                if event.device_name().is_some() {
                    got_insn = true;
                    tracing::info!(
                        "Received trace event: PC=0x{:X}, Disas={}",
                        event.id(),
                        event.device_name().unwrap()
                    );
                    break;
                }
            }
        }
    }

    assert!(got_insn, "TCG Tracer did not stream instruction execution!");
    Ok(())
}

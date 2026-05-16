use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

use sha2::{Digest, Sha256};

async fn run_ping_pong_test(transport: &str) -> Result<Vec<String>> {
    let outputs = Arc::new(Mutex::new(Vec::new()));
    let outputs_clone = outputs.clone();

    VirtmcuTestEnv::builder()
        .with_transport_override(transport)
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/reference_ping_pong/pinger.elf")
                .with_yaml_path("worlds/reference_ping_pong.yml"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/reference_ping_pong/ponger.elf")
                .with_yaml_path("worlds/reference_ping_pong.yml"),
        )
        .with_timeout(60)
        .run_test(move |env| {
            Box::pin(async move {
                env.wait_for_output(0, "Node 0: Pinger starting").await?;
                env.wait_for_output(0, "Node 0: Ping sent, waiting for Pong...")
                    .await?;
                env.wait_for_output(1, "Node 1: Ponger starting").await?;
                env.step_clock(50_000_000, 10_000_000).await?;

                env.wait_for_output(1, "Node 1: Ping received!").await?;
                env.wait_for_output(0, "Node 0: Pong received! Test complete.")
                    .await?;
                env.wait_for_output(1, "Node 1: Pong sent! Test complete.")
                    .await?;

                let out0 = env.uart_buffer(0).await;
                let out1 = env.uart_buffer(1).await;
                *outputs_clone.lock().await = vec![out0, out1];

                Ok(())
            })
        })
        .await?;

    let res = outputs.lock().await.clone();
    Ok(res)
}

fn hash_output(output: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(output.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_reference_ping_pong_zenoh() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let _ = run_ping_pong_test("zenoh").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_reference_ping_pong_unix() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let _ = run_ping_pong_test("unix").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_reference_ping_pong_transport_parity() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    println!("Running Zenoh transport test...");
    let zenoh_out = run_ping_pong_test("zenoh")
        .await
        .expect("Zenoh test failed");

    println!("Running Unix transport test...");
    let unix_out = run_ping_pong_test("unix").await.expect("Unix test failed");

    let zenoh_h0 = hash_output(&zenoh_out[0]);
    let zenoh_h1 = hash_output(&zenoh_out[1]);
    let unix_h0 = hash_output(&unix_out[0]);
    let unix_h1 = hash_output(&unix_out[1]);

    println!("Node 0 (Zenoh) Hash: {}", zenoh_h0);
    println!("Node 0 (Unix)  Hash: {}", unix_h0);
    println!("Node 1 (Zenoh) Hash: {}", zenoh_h1);
    println!("Node 1 (Unix)  Hash: {}", unix_h1);

    assert_eq!(
        zenoh_h0, unix_h0,
        "Node 0 output mismatch (hash) between transports!\nZenoh: {}\nUnix: {}",
        zenoh_out[0], unix_out[0]
    );
    assert_eq!(
        zenoh_h1, unix_h1,
        "Node 1 output mismatch (hash) between transports!\nZenoh: {}\nUnix: {}",
        zenoh_out[1], unix_out[1]
    );

    Ok(())
}

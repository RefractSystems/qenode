use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_complex_board_wireless() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/complex_board/radio_test.elf")
                .with_yaml_path("tests/fixtures/guest_apps/complex_board/board.yaml")
                .orchestrated(false),
        )
        .with_timeout(60)
        .build()
        .await?;

    // Wait for the firmware to start
    env.wait_for_output(0, "Radio test firmware starting...")
        .await?;

    // Wait for the firmware to send its own packet
    env.wait_for_output(0, "Packet sent successfully.").await?;

    // Now inject a packet back to the radio
    // The firmware expects a packet and prints "Received packet!"
    let mut dummy_packet = vec![
        0x41, 0x88, /* Frame Control: Data, Ack Request, Pan ID Compression */
        0x02, /* Sequence Number */
        0xCD, 0xAB, /* Dest PAN ID: 0xABCD */
        0x34, 0x12, /* Dest Addr: 0x1234 */
        0x78, 0x56, /* Source Addr: 0x5678 */
    ];
    dummy_packet.extend_from_slice(b"HELLO FROM TEST");

    let radio_rx_topic = "sim/rf/ieee802154/0/rx";

    // Create a Zenoh packet with header. Use a future vtime to ensure it's processed.
    let now_vtime = 500_000_000;
    let payload = virtmcu_api::encode_rf802154_frame(
        now_vtime,
        0,
        &dummy_packet,
        -50, // rssi
        255, // lqi
    );

    env.session()
        .put(radio_rx_topic, payload)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to publish to radio: {}", e))?;

    // Advance clock to allow processing (we need to advance past the injected vtime)
    env.step_clock(1_000_000_000, 1_000_000).await?;

    env.wait_for_output(0, "Received packet!").await?;
    env.wait_for_output(0, "HELLO FROM TEST").await?;

    Ok(())
}

use anyhow::Result;
use virtmcu_api::lin_generated::virtmcu::lin::LinMessageType;
use virtmcu_test_runner::{monitors::LinMonitor, NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_lin_lpuart() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let topic_prefix = "sim/lin";

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/lin_bridge/lin_echo.elf")
                .with_yaml_path("tests/fixtures/guest_apps/lin_bridge/lin_test.yml")
                .add_qemu_arg("-device")
                .add_qemu_arg("s32k144-lpuart,node=0"),
        )
        .with_variable("LIN_TOPIC", topic_prefix)
        .with_timeout(60)
        .build()
        .await?;

    let tx_topic = format!("{}/0/tx", topic_prefix);
    let rx_topic = format!("{}/0/rx", topic_prefix);
    let monitor = LinMonitor::new(&env.session(), &tx_topic).await?;
    env.step_clock(5_000_000, 1_000_000).await?;

    monitor
        .publish(&rx_topic, 1_000_000, LinMessageType::Data, Some(b"X"))
        .await?;
    env.step_clock(5_000_000, 1_000_000).await?;

    monitor
        .publish(&rx_topic, 6_000_000, LinMessageType::Break, None)
        .await?;
    env.step_clock(5_000_000, 1_000_000).await?;

    let found = monitor
        .wait_for_responses(30, |msgs| {
            let mut found_x = false;
            let mut found_b = false;
            for (msg_type, data) in msgs {
                if *msg_type == LinMessageType::Data {
                    if data == b"X" {
                        found_x = true;
                    }
                    if data == b"B" {
                        found_b = true;
                    }
                }
            }
            found_x && found_b
        })
        .await?;
    assert!(found, "Failed to receive Echo responses");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_lin_stress() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let topic_prefix = "sim/lin";

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/lin_bridge/lin_echo.elf")
                .with_yaml_path("tests/fixtures/guest_apps/lin_bridge/lin_test.yml")
                .add_qemu_arg("-device")
                .add_qemu_arg("s32k144-lpuart,node=0"),
        )
        .with_variable("LIN_TOPIC", topic_prefix)
        .with_timeout(60)
        .build()
        .await?;

    let tx_topic = format!("{}/0/tx", topic_prefix);
    let rx_topic = format!("{}/0/rx", topic_prefix);
    let monitor = LinMonitor::new(&env.session(), &tx_topic).await?;
    env.step_clock(5_000_000, 1_000_000).await?;

    let iters = if std::env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1" {
        20
    } else {
        100
    };
    for i in 0..iters {
        monitor
            .publish(&rx_topic, i * 1_000_000, LinMessageType::Data, Some(b"S"))
            .await?;
        env.step_clock(1_000_000, 1_000_000).await?;
    }
    let found = monitor
        .wait_for_responses(30, |msgs| msgs.len() >= iters as usize)
        .await?;
    assert!(found, "Failed to receive all stress responses");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_lin_multi_node() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let topic_prefix = "sim/lin";

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/lin_bridge/lin_master.elf")
                .with_yaml_path("tests/fixtures/guest_apps/lin_bridge/lin_test.yml")
                .add_qemu_arg("-device")
                .add_qemu_arg("s32k144-lpuart,node=0"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/lin_bridge/lin_slave.elf")
                .with_yaml_path("tests/fixtures/guest_apps/lin_bridge/lin_test.yml")
                .add_qemu_arg("-device")
                .add_qemu_arg("s32k144-lpuart,node=1"),
        )
        .with_variable("LIN_TOPIC", topic_prefix)
        .with_timeout(20)
        .build()
        .await?;

    env.step_clock(500_000_000, 5_000_000).await?;
    Ok(())
}

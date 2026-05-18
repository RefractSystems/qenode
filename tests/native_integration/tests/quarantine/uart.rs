use anyhow::Result;
use virtmcu_test_runner::{monitors::ChardevMonitor, NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_uart_stress() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let topic_prefix = "sim/chardev/test_uart_stress";

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/uart_echo/echo.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb")
                .add_qemu_arg("-chardev")
                .add_qemu_arg(&format!(
                    "virtmcu,id=chr0,node=0,router={{ROUTER_ENDPOINT}},topic={}",
                    topic_prefix
                ))
                .add_qemu_arg("-serial")
                .add_qemu_arg("chardev:chr0"),
        )
        .with_timeout(15)
        .run_test(|env| Box::pin(async move {
            let tx_topic = format!("{}/0/tx", topic_prefix);
            let rx_topic = format!("{}/0/rx", topic_prefix);
            let monitor = ChardevMonitor::new(&env.session(), &tx_topic).await?;

            let mut current_vtime = 0;
            // Step clock until welcome message
            for _ in 0..50 {
                env.step_clock(10_000_000, 1_000_000).await?;
                current_vtime += 10_000_000;
                let buf = monitor.captured_text.lock().unwrap().clone();
                if buf.contains("Interactive UART Echo Ready.") {
                    break;
                }
            }
            monitor.clear().await;

            let burst = b"BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_BURST_TEST_END\n";
            monitor
                .publish(&rx_topic, current_vtime + 1_000_000, burst)
                .await?;

            // We need to continuously step the clock while waiting for the echo.
            let mut found = false;
            let mut final_buf = String::new();
            let burst_str = std::str::from_utf8(burst).unwrap();
            let iters = if std::env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1" {
                50
            } else {
                500
            };
            for _ in 0..iters {
                env.step_clock(10_000_000, 1_000_000).await?;

                let buf = monitor.captured_text.lock().unwrap().clone();
                if buf.contains(burst_str) {
                    found = true;
                    break;
                }
                final_buf = buf;
            }

            assert!(
                found,
                "Did not receive full echo. Current buffer: {:?}",
                final_buf
            );

            Ok(())
        })).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_multi_node_uart() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let topic0 = "sim/chardev/test_uart_multi_0";
    let topic1 = "sim/chardev/test_uart_multi_1";

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/uart_echo/echo.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb")
                .add_qemu_arg("-chardev")
                .add_qemu_arg(&format!(
                    "virtmcu,id=chr0,node=0,router={{ROUTER_ENDPOINT}},topic={}",
                    topic0
                ))
                .add_qemu_arg("-serial")
                .add_qemu_arg("chardev:chr0"),
        )
        .add_node(
            NodeConfig::new(1)
                .with_firmware_path("tests/fixtures/guest_apps/uart_echo/echo.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb")
                .add_qemu_arg("-chardev")
                .add_qemu_arg(&format!(
                    "virtmcu,id=chr0,node=1,router={{ROUTER_ENDPOINT}},topic={}",
                    topic1
                ))
                .add_qemu_arg("-serial")
                .add_qemu_arg("chardev:chr0"),
        )
        .with_timeout(15)
        .run_test(|env| {
            Box::pin(async move {
                let rx0_topic = format!("{}/0/rx", topic0);
                let tx0_topic = format!("{}/0/tx", topic0);
                let rx1_topic = format!("{}/1/rx", topic1);
                let tx1_topic = format!("{}/1/tx", topic1);

                let monitor0 = ChardevMonitor::new(&env.session(), &tx0_topic).await?;
                let monitor1 = ChardevMonitor::new(&env.session(), &tx1_topic).await?;

                // In the python test it had a bridge script running, but here we can just bridge via code.
                let session_clone = env.session();
                let rx1_topic_clone = rx1_topic.clone();
                let sub = env.safe_subscribe(&tx0_topic).await.unwrap();
                tokio::spawn(async move {
                    while let Ok(sample) = sub.recv_async().await {
                        let _ = session_clone
                            .put(&rx1_topic_clone, sample.payload().to_bytes().into_owned())
                            .await;
                    }
                });

                // Step clock until welcome message
                for _ in 0..50 {
                    env.step_clock(10_000_000, 1_000_000).await?;
                    let buf0 = monitor0.captured_text.lock().unwrap().clone();
                    let buf1 = monitor1.captured_text.lock().unwrap().clone();
                    if buf0.contains("Interactive UART Echo Ready.")
                        && buf1.contains("Interactive UART Echo Ready.")
                    {
                        break;
                    }
                }
                monitor0.clear().await;
                monitor1.clear().await;

                // Send PING to node 0, it should be bridged to node 1
                monitor0
                    .publish(&rx0_topic, 500_000_000 + 10_000_000, b"PING")
                    .await?;

                env.step_clock(50_000_000, 5_000_000).await?;

                let found0 = monitor0.wait_for_pattern(5, "PING").await?;
                let found1 = monitor1.wait_for_pattern(5, "PING").await?;

                assert!(found0, "Node 0 did not echo PING");
                assert!(found1, "Node 1 did not echo PING via bridge");

                Ok(())
            })
        })
        .await?;

    Ok(())
}

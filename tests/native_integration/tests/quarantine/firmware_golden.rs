#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"

use anyhow::{Context, Result};
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_firmware_golden_cortex_a15_virt() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let expected_golden = include_str!("../../firmware/cortex-a15-virt/golden_uart.txt");

    // We only want the non-comment lines
    let expected_lines: Vec<&str> = expected_golden
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect();
    let expected_clean = expected_lines.join("\n");

    let expected_clean = expected_clean.trim();

    let env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/firmware/cortex-a15-virt/echo.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb")
                .orchestrated(false),
        )
        .with_timeout(10)
        .build()
        .await
        .context("Failed to build test environment")?;

    // The old bash script ran for 3 seconds. Here we wait for the target string.
    // Wait for the prompt which is the last line of the expected output.
    let target_string = expected_lines
        .last()
        .expect("Golden file must have non-comment lines")
        .to_string();
    let expected_clean_owned = expected_clean.to_string();

    env.run_test(|env| {
        Box::pin(async move {
            env.wait_for_output(0, &target_string)
                .await
                .context(format!("Failed to reach '{}'", target_string))?;

            // Fetch the full UART trace
            let actual_uart = env.uart_buffer(0).await;

            // Normalize the trace: strip \r
            let actual_clean = actual_uart.replace("\r", "");
            let actual_clean = actual_clean.trim();

            assert_eq!(
                actual_clean, expected_clean_owned,
                "UART output deviates from validated silicon baseline"
            );

            Ok(())
        })
    })
    .await
}

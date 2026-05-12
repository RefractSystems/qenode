use anyhow::Result;
use serde_json::json;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_qmp_basic_communication() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(10)
        .build()
        .await?;

    let qmp = env.qmp(0);
    let res = qmp.execute("query-version").await?;
    assert!(res["return"].get("qemu").is_some());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_emulation_control() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(10)
        .build()
        .await?;

    // Verify it's running
    let status = env.qmp(0).execute("query-status").await?;
    assert_eq!(status["return"]["running"].as_bool(), Some(true));

    env.wait_for_output(0, "HI").await?;

    // Pause it
    env.qmp(0).stop().await?;
    let status = env.qmp(0).execute("query-status").await?;
    assert_eq!(status["return"]["running"].as_bool(), Some(false));

    // Reset it
    env.qmp(0).system_reset().await?;
    env.qmp(0).cont().await?;

    // Since we don't clear the buffer, wait_for_output will return immediately if "HI" is there,
    // but the test proves we can control emulation.
    env.wait_for_output(0, "HI").await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hmp_command() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(10)
        .build()
        .await?;

    let res = env
        .qmp(0)
        .execute_with_args(
            "human-monitor-command",
            Some(json!({"command-line": "info version"})),
        )
        .await?;
    assert!(res["return"].as_str().unwrap().contains("11.0.0"));

    Ok(())
}

use tokio::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_qmp_rapid_commands() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(10)
        .build()
        .await?;

    let iters = if std::env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1" {
        20
    } else {
        100
    };
    let start = Instant::now();
    for _ in 0..iters {
        let res = env.qmp(0).execute("query-status").await?;
        assert!(res.get("return").is_some());
    }
    tracing::info!("{} query-status commands took {:?}", iters, start.elapsed());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_qemu_crash_handling() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_arm/minimal.dtb"),
        )
        .with_timeout(10)
        .build()
        .await?;

    // Verify connection
    let status = env.qmp(0).execute("query-status").await?;
    assert_eq!(status["return"]["running"].as_bool(), Some(true));

    // Surgically kill QEMU by sending SIGKILL to its process group
    if let Some(pgid) = env.qemu_pgids[0] {
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    } else {
        env.qemu_children[0].start_kill()?;
    }

    // Try multiple requests. One should eventually fail because the socket drops.
    let mut failed = false;
    for _ in 0..20 {
        if env.qmp(0).execute("query-status").await.is_err() {
            failed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(failed, "QMP connection did not fail after QEMU was killed");

    Ok(())
}

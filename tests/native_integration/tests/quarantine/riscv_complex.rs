use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_riscv_boot() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_arch("riscv64")
                .with_firmware_path("tests/fixtures/guest_apps/boot_riscv/hello.elf")
                .with_dtb_path("tests/fixtures/guest_apps/boot_riscv/minimal.dtb")
                .add_qemu_arg("-m")
                .add_qemu_arg("512M")
                .add_qemu_arg("-bios")
                .add_qemu_arg("none")
                .orchestrated(false),
        )
        .with_timeout(10)
        .build()
        .await?;

    env.wait_for_output(0, "HI RV").await?;

    Ok(())
}

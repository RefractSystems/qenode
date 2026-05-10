use anyhow::Result;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::Duration;
use tracing::info;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_irq_stress() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // 1. Setup the environment with the adapter socket
    let socket_path = "/tmp/stress_adapter.sock";

    // We start QEMU with a custom DTS mapped to the mmio-socket-bridge
    let dts = format!(
        r#"
/dts-v1/;
/ {{
    model = "virtmcu-stress-test";
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;

    qemu_sysmem: qemu_sysmem {{ compatible = "qemu:system-memory"; phandle = <0x01>; }};
    chosen {{}};
    memory@40000000 {{
        compatible = "qemu-memory-region";
        qemu,ram = <0x01>;
        container = <0x01>;
        reg = <0x0 0x40000000 0x0 0x10000000>;
    }};
    cpus {{
        #address-cells = <1>;
        #size-cells = <0>;
        cpu@0 {{
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
            memory = <0x01>;
        }};
    }};
    uart0@9000000 {{
        compatible = "pl011";
        reg = <0x0 0x09000000 0x0 0x1000>;
        chardev = <0x00>;
    }};
    bridge@50000000 {{
        compatible = "mmio-socket-bridge";
        reg = <0x0 0x70000000 0x0 0x1000>;
        socket-path = "{}";
        region-size = <0x1000>;
    }};
}};"#,
        socket_path
    );

    let asm = r#"
.global _start
_start:
    ldr r0, =0x70000000
    ldr r2, =0x186A0        @ 100,000 iterations
loop:
    str r2, [r0]            @ Write to bridge
    ldr r1, [r0]            @ Read back
    subs r2, r2, #1
    bne loop
    
    @ Success
    ldr r3, =0x09000000
    mov r4, #'O'
    str r4, [r3]
    mov r4, #'K'
    str r4, [r3]
    mov r4, #'\n'
    str r4, [r3]
1:  b 1b
"#;

    // Build the adapter
    info!("Building stress adapter...");
    let mut build_cmd = Command::new("cargo");
    build_cmd.args(["build", "--release", "-p", "stress_adapter"]);
    let status = build_cmd.status().await?;
    assert!(status.success(), "Failed to build stress adapter");

    // Start stress adapter via cargo
    let mut adapter_cmd = Command::new("cargo");
    adapter_cmd
        .args([
            "run",
            "--release",
            "-p",
            "stress_adapter",
            "--",
            socket_path,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut adapter = adapter_cmd.spawn()?;

    let adapter_stdout = adapter.stdout.take().unwrap();
    let mut adapter_reader = BufReader::new(adapter_stdout).lines();
    tokio::spawn(async move {
        while let Ok(Some(line)) = adapter_reader.next_line().await {
            info!("Adapter: {}", line);
        }
    });

    // Wait for socket
    let mut found = false;
    let socket_path_buf = std::path::PathBuf::from(socket_path);
    for _ in 0..50 {
        if socket_path_buf.exists() {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(found, "Stress adapter failed to create socket");

    // Start environment
    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_asm(asm)
                .with_dts_content(&dts)
                .orchestrated(false),
        )
        .with_timeout(60)
        .build()
        .await?;

    let success = env.wait_for_output_passive(0, "OK").await;

    // Teardown async
    env.teardown().await;
    let _ = adapter.kill().await;

    assert!(success.is_ok(), "Test timed out without receiving OK");
    Ok(())
}

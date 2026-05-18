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

    // We start QEMU with a custom DTS mapped to the mmio-socket-bridge.
    // We use {TMP_DIR} which is automatically substituted by VirtmcuTestEnv.
    let dts = r#"
/dts-v1/;
/ {
    model = "virtmcu-stress-test";
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;

    qemu_sysmem: qemu_sysmem { compatible = "qemu:system-memory"; phandle = <0x01>; };
    chosen {};
    memory@40000000 {
        compatible = "qemu-memory-region";
        qemu,ram = <0x01>;
        container = <0x01>;
        reg = <0x0 0x40000000 0x0 0x10000000>;
    };
    cpus {
        #address-cells = <1>;
        #size-cells = <0>;
        cpu@0 {
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
            memory = <0x01>;
        };
    };
    uart0@9000000 {
        compatible = "pl011";
        reg = <0x0 0x09000000 0x0 0x1000>;
        chardev = <0x00>;
    };
    bridge@50000000 {
        compatible = "mmio-socket-bridge";
        reg = <0x0 0x70000000 0x0 0x1000>;
        socket-path = "{TMP_DIR}/stress_adapter.sock";
        region-size = <0x1000>;
    };
};"#;

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

    // Start environment
    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_asm(asm)
                .with_dts_content(dts)
                .orchestrated(false),
        )
        .with_timeout(60)
        .run_test(|env| {
            Box::pin(async move {
                let socket_path = env.tmp_path("stress_adapter.sock");
                let socket_path_str = socket_path.to_str().expect("Valid UTF-8 path");

                // Use the artifact path from build.rs if available, otherwise find it dynamically
                let adapter_bin = if let Some(path) = option_env!("STRESS_ADAPTER_BIN") {
                    info!("Using stress adapter artifact from build.rs: {}", path);
                    std::path::PathBuf::from(path)
                } else {
                    let bin = env.find_binary("stress_adapter")?;
                    info!("Using stress adapter found via find_binary: {:?}", bin);
                    bin
                };

                if !adapter_bin.exists() {
                    panic!(
                        "Stress adapter binary not found at {:?}. \
                         Please build it with 'cargo build -p stress_adapter --release' \
                         or 'make build-test-artifacts'.",
                        adapter_bin
                    );
                }

                // Start stress adapter directly
                info!("Starting stress adapter at {}...", socket_path_str);
                let mut adapter_cmd = Command::new(&adapter_bin);
                adapter_cmd
                    .args([socket_path_str])
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

                // Wait for socket to be created by the adapter
                let mut found = false;
                for _ in 0..50 {
                    if socket_path.exists() {
                        found = true;
                        break;
                    }
                    // virtmcu-allow: test_sleep reasoning="wait for socket creation"
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                assert!(
                    found,
                    "Stress adapter failed to create socket at {}",
                    socket_path_str
                );

                // Register adapter for automatic teardown
                env.register_child(adapter);

                let success = env.wait_for_output_passive(0, "OK").await;
                assert!(success.is_ok(), "Test timed out without receiving OK");
                Ok(())
            })
        })
        .await
}

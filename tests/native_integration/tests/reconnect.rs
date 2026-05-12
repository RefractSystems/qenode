use anyhow::Result;
use tokio::time::Duration;
use tracing::info;
use virtmcu_api::{FlatBufferStructExt, SyscMsg, SYSC_MSG_RESP};
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_reconnect() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let asm = r#"
.global _start
_start:
    ldr r0, =0x70000000
loop:
    ldr r1, [r0]            /* This will fail initially, then succeed after reconnect */
    cmp r1, #0x42
    bne loop
    
    /* Success */
    ldr r3, =0x09000000
    mov r4, #'O'
    str r4, [r3]
    mov r4, #'K'
    str r4, [r3]
    mov r4, #'\n'
    str r4, [r3]
1:  b 1b
"#;

    let dts = r#"
/dts-v1/;
/ {
    model = "virtmcu-reconnect-test";
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
        socket-path = "{TMP_DIR}/reconnect.sock";
        region-size = <0x1000>;
        reconnect-ms = <500>;
    };
};"#;

    // Start QEMU (adapter not yet started)
    info!("Starting QEMU (expecting connection errors initially)...");

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_asm(asm)
                .with_dts_content(dts)
                .orchestrated(false),
        )
        .with_timeout(10)
        .run_test(|env| {
            Box::pin(async move {
                let socket_path = env.tmp_path("reconnect.sock");
                let socket_path_str = socket_path.to_str().expect("Valid UTF-8 path").to_string();

                // Wait a bit, then start mock adapter
                tokio::time::sleep(Duration::from_secs(2)).await;
                info!("Starting mock adapter at {}...", socket_path_str);

                let adapter_handle = tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    use tokio::net::UnixListener;

                    let listener = UnixListener::bind(&socket_path_str)?;
                    let (mut stream, _) = listener.accept().await?;

                    // Handshake
                    let mut hs_buf = [0u8; 8];
                    stream.read_exact(&mut hs_buf).await?;
                    stream.write_all(&hs_buf).await?;

                    // Handle requests
                    let mut req_buf = [0u8; 32];
                    while stream.read_exact(&mut req_buf).await.is_ok() {
                        let resp = SyscMsg::new(SYSC_MSG_RESP, 0, 0x42);
                        stream.write_all(resp.pack()).await?;
                    }
                    Ok::<(), anyhow::Error>(())
                });

                let success = env.wait_for_output_passive(0, "OK").await;
                adapter_handle.abort();

                assert!(success.is_ok(), "Test timed out waiting for OK");
                Ok(())
            })
        })
        .await
}

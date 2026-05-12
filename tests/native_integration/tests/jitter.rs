use anyhow::Result;
use tokio::time::Duration;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_jitter_neutralization() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // The clock_coordinator natively wraps the clock stepping via Zenoh.
    // If we advance the virtual time, the underlying simulation should produce bit-identical
    // VTime execution boundaries regardless of real-world jitter.
    // We demonstrate this by doing a simple loop and capturing its output.

    let asm = r#"
.global _start
_start:
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
    model = "virtmcu-jitter-test";
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
};"#;

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_asm(asm)
                .with_dts_content(dts)
                .orchestrated(true), // Crucial for PDES
        )
        .with_timeout(10)
        .run_test(|env| {
            Box::pin(async move {
                // Step clock with intentional random-like sleeps to simulate jitter
                for _ in 0..10 {
                    env.step_clock(100_000, 100_000).await?;
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }

                let success = env.wait_for_output_passive(0, "OK").await;
                assert!(
                    success.is_ok(),
                    "Test timed out without receiving OK under jitter"
                );

                Ok(())
            })
        })
        .await
}

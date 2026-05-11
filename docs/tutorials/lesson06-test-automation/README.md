# Lesson 6: Emulation Test Automation with VirtmcuTestEnv

In this lesson, you will learn how to automate the testing of your firmware and virtual hardware using the **VirtmcuTestEnv** Rust framework. We will explore how to orchestrate QEMU nodes, monitor UART output, and advance virtual time deterministically.

## Concepts

### Deterministic Orchestration
Unlike traditional testing where you might use shell scripts to launch QEMU, `virtmcu` uses a Rust-based environment builder. This ensures:
- **Synchronization**: All nodes start frozen (`-S`) and are unfrozen simultaneously only after all connections are established.
- **Resource Management**: QEMU processes and sockets are automatically cleaned up using RAII (Rust's `Drop` trait).
- **Virtual Time**: Time only advances when your test code explicitly calls `env.step_clock()`.

---

## The VirtmcuTestEnv Builder

The primary entry point for integration tests is `VirtmcuTestEnv::builder()`.

### Key Components:
- **NodeConfig**: Defines the firmware, DTB, and QEMU arguments for a single node.
- **ChardevMonitor**: A helper to subscribe to UART topics and search for patterns.
- **env.step_clock(ns, quantum)**: Advances the simulation by a specific number of nanoseconds.

---

## Hands-on: Writing a Rust Test

Integration tests live in `tests/native_integration/tests/`. They are standard Rust `#[tokio::test]` functions.

### The Test Case (`tests/native_integration/tests/my_test.rs`)

```rust
use anyhow::Result;
use virtmcu_test_runner::{monitors::ChardevMonitor, NodeConfig, VirtmcuTestEnv};

#[tokio::test]
async fn test_hello_world() -> Result<()> {
    let topic = "sim/chardev/hello";

    VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("hello.elf")
                .with_dtb_path("board.dtb")
                .add_qemu_arg("-chardev")
                .add_qemu_arg(&format!("virtmcu,id=chr0,node=0,router={ROUTER_ENDPOINT},topic={}", topic))
                .add_qemu_arg("-serial")
                .add_qemu_arg("chardev:chr0"),
        )
        .run_test(|env| Box::pin(async move {
            let tx_topic = format!("{}/0/tx", topic);
            let monitor = ChardevMonitor::new(&env.session(), &tx_topic).await?;

            // 1. Advance time and wait for the firmware to print "HI"
            let mut found = false;
            for _ in 0..10 {
                env.step_clock(10_000_000, 1_000_000).await?;
                if monitor.captured_text.lock().unwrap().contains("HI") {
                    found = true;
                    break;
                }
            }
            
            assert!(found, "Firmware did not print HI");
            Ok(())
        })).await;

    Ok(())
}
```

---

## Running the Tests

To run your integration tests, use `cargo test`:

```bash
# Run all integration tests
cargo test -p native-integration

# Run a specific test with logging enabled
RUST_LOG=info cargo test -p native-integration -- --nocapture
```

## Summary
- **VirtmcuTestEnv** provides a type-safe, asynchronous environment for simulation testing.
- **Deterministic Stepping** eliminates flakiness by tying test progress to virtual time, not wall-clock time.
- **Rust Integration** allows you to use the full power of the Rust ecosystem (tracing, anyhow, tokio) in your test suites.

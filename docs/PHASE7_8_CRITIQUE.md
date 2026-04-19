# Phase 7 & 8 Combined Critique (Clock Sync & Interactive UART)

## 1. What went wrong / What was missed?
- **UART Rx Overruns:** The `zenoh-chardev` (Phase 8) writes data into QEMU directly via `qemu_chr_be_write`. QEMU's PL011 UART model has a hardware FIFO of exactly 32 bytes. Any Zenoh packet larger than 32 bytes instantly overruns the guest FIFO, silently dropping the remaining bytes. This occurs frequently when users "paste" text into an interactive terminal or when automated test scripts send multi-line commands in a single Zenoh packet.
- **Clock Stall Timeout Vulnerability:** The `zenoh-clock` module (Phase 7) implements a `stall_timeout_ms` (default 5000ms). If a Zenoh router is slow, or another node pauses in a debugger, the clock times out and breaks the simulation loop silently (or causes a deadlock during shutdown). We lacked a dedicated test proving that a prolonged stall cleanly aborts the process without locking up the BQL or memory.

## 2. Un-tested Assumptions & Assertions
- **Assumption - BQL on Chardev writes:** We assume that writing to `qemu_chr_be_write` is safe from Zenoh's async worker threads. This was recently fixed with `virtmcu_bql_lock()`, but we still assume `qemu_chr_be_write` can handle arbitrary-length slices without returning errors.
- **Assumption - Time Monotonicity:** `cpu_clock_offset` manipulation assumes delta increments strictly move time forward. However, we did not add explicit assertions validating that `quantum_target >= current_vtime` before updating the timers state.
- **Assumption - Host Clock Stability:** The timeout logic uses `std::time::Instant::now()`. If the host OS is suspended (e.g., laptop lid closed) during a wait cycle, `Instant` elapsed times can wildly jump, triggering a false-positive simulator stall.

## 3. What should be done better?
- **Chardev Chunking & Backpressure**: Update `zenoh-chardev` to query `qemu_chr_be_can_write` before pushing data. If the guest UART FIFO is full, the device should queue the remainder of the packet and retry later (using a timer or a guest-read callback), matching true hardware flow-control.
- **Stall Behavior Testing**: Create a new test case (`test/phase7/clock_stall_test.sh`) that specifically holds the `sim/clock/advance/0` queryable open for >5000ms and validates QEMU handles the timeout cleanly.
- **UART Flood Stress Testing**: Create a high-baud UART stress test (`test/phase8/uart_flood_test.sh`) that transmits massive chunks of text to verify the chunking / backpressure logic prevents dropped characters without crashing QEMU.
- **Improved Test Coverage**: Decouple the timing structs into `virtmcu-api` and add `#[test]` unit tests for the core clock state machines, increasing Phase 7 & 8 test coverage.

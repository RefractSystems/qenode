# Phase 18 & 19 Combined Critique (Rust Migration)

## 1. What went wrong / What was missed?
- **Silently Broken Sub-systems:** The `zenoh-802154` device was entirely stubbed out during the Phase 19 port because its C shim was complex, but the integration test `test/phase14/smoke_test.sh` only tests `yaml2qemu` CLI output, not runtime behavior.
- **Race Conditions across FFI:** The Zenoh async worker threads do not inherently hold the QEMU Big QEMU Lock (BQL). Devices like `zenoh-chardev` and `zenoh-ui` had their subscribers ported naively, triggering QEMU MMIO or state changes directly from the Zenoh worker without `virtmcu_bql_lock()`.
- **Determinism Jitter:** In `slaved-suspend` mode, the `zenoh-clock` QEMU Mutex/Condvar handshake added sub-millisecond context switch latencies that accumulated, breaking perfect cycle-accuracy across repeated test runs.

## 2. Un-tested Assumptions & Assertions
- **Assumption: Callbacks don't panic.** If Rust panics across an `extern "C"` boundary, it's UB and crashes QEMU. We assume none of the MMIO reads/writes panic, but we don't catch unwinds or use `#![panic=abort]`.
- **Assumption: Lifecycles are clean.** We assume `instance_finalize` correctly `drop`s the `Box::into_raw` state. But there are no tests for hot-unplugging devices to verify memory is reclaimed cleanly.
- **Assumption: Network Queues are bounded.** `crossbeam_channel::unbounded()` was used for `zenoh-netdev` to escape the BQL. If a Zenoh publisher floods the topic faster than the QEMU CPU processes interrupts, the QEMU process will eventually OOM. This was not tested.
- **Assumption: Endianness is handled.** We assume MMIO ops correctly use `DEVICE_LITTLE_ENDIAN`, but we do not enforce that our Rust struct byte-layouts or manual shifts are verified against simulated guest behavior under stress.

## 3. What should be done better?
- **OOM Prevention**: The lock-free MPSC channel in `zenoh-netdev` must be changed from `unbounded` to `bounded` (e.g., `1024` packets) to backpressure Zenoh workers instead of crashing QEMU on network floods.
- **Improved Coverage**: Add explicit Rust unit tests (`#[cfg(test)]`) inside the `hw/rust/*` crates to test the internal logic, like quantum bounds in `zenoh-clock` or packet sorting in `zenoh-netdev`, avoiding reliance entirely on `make test-integration`.
- **Flood Testing**: Implement a stress test (`test/phase19/netdev_flood_test.py`) to actively try and crash QEMU by flooding the network interface.

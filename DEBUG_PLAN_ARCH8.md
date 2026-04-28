# Debugging Plan for ARCH-8 (TA/Coordinator Synchronization Protocol)

To avoid getting stuck on ARCH-8 or similar architectural synchronization tasks, we will follow this structured debugging and iteration strategy.

## 1. Fast Iteration Loop
- **Targeted Rebuilds**: Instead of `make build`, use targeted ninja commands to rebuild only the modified plugin:
  ```bash
  ninja -C third_party/qemu/build-virtmcu hw-virtmcu-clock.so
  ```
- **Focused Testing**: Run only the relevant test with full output:
  ```bash
  pytest -v -s tests/test_arch8_coordinator_sync.py
  ```

## 2. Visibility & Observability
- **Rust Debug Logging**: Use `virtmcu_qom::sim_info!` or `sim_trace!` with an `ARCH-8:` prefix in the Rust code to track internal state transitions.
  - Key points: Sending 'done' signal, waiting for 'start', receiving 'start'.
- **Python Debug Logging**: Use `print(f"DEBUG: ...")` in the test's `coordinator_loop` and main body.
  - Key points: Receiving 'done', sleeping (delivery delay), sending 'start', VTA step timing.
- **QEMU STDERR Monitoring**: ALWAYS check the first few lines of QEMU output for "failed to open module" or "undefined symbol". These errors prevent the device from ever being realized, leading to confusing EOF errors in QMP.

## 3. Mandatory FFI/Symbol Checks
- Any function called from QEMU's C code or another DSO must be marked `#[no_mangle] extern "C"`.
- If a plugin fails to load, verify its symbols using:
  ```bash
  QEMU_MODULE_DIR=third_party/qemu/build-virtmcu ./third_party/qemu/build-virtmcu/install/bin/qemu-system-arm -device help | grep <device-name>
  ```

## 4. Test Infrastructure Hygiene
- **Subscriber Cleanup**: Ensure all Zenoh subscribers are stored in variables and explicitly undeclared in `finally` blocks to avoid resource leaks and interference between test runs.
- **Race Condition Prevention**: Reset asyncio events *before* the action that might trigger them.
- **Timing Robustness**: Use `time.monotonic()` for all wall-clock duration measurements.

## 5. Implementation Checklist for ARCH-8
- [x] Fix `#[no_mangle]` for `clock_cpu_halt_cb` in `hw/rust/backbone/clock/src/lib.rs`.
- [x] Fix `NameError` in `tests/test_arch8_coordinator_sync.py`.
- [ ] Implement coordination logic in `UnixSocketClockTransport` in `hw/rust/common/virtmcu-api/src/lib.rs` (if applicable).
- [ ] Verify that `coordinated=true` is correctly propagated from Device Tree to the Rust state.

## 6. Learning from Failure
- If the test passes but the timing is wrong, check if `is_coordinated` is actually `true` in the Rust backend.
- If the test hangs, check if the coordinator is actually sending the `start` signal to the correct topic (`sim/clock/start/{node_id}`).

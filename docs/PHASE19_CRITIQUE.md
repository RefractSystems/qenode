# Phase 19 Critique

## 1. What went wrong / What was missed?
- **Silently Broken Sub-systems:** The `zenoh-802154` device was entirely stubbed out during the Phase 19 port because its C shim was complex, but the integration test `test/phase14/smoke_test.sh` only tests `yaml2qemu` CLI output, not runtime behavior. This allowed the stub to slip into `main` undetected.
- **Race Conditions across FFI:** The Zenoh async worker threads do not inherently hold the QEMU Big QEMU Lock (BQL). Devices like `zenoh-chardev` and `zenoh-ui` had their subscribers ported naively, triggering QEMU MMIO or state changes directly from the Zenoh worker without `virtmcu_bql_lock()`.

## 2. Un-tested Assumptions & Assertions
- **Assumption: Callbacks don't panic.** If Rust panics across an `extern "C"` boundary, it's UB and crashes QEMU. We assume none of the MMIO reads/writes panic, but we don't catch unwinds or use `#![panic=abort]`.
- **Assumption: BQL is held.** We assume that callbacks like `qemu_set_irq` or `qemu_chr_be_write` are safe from anywhere, which is false. We do not use `assert!(virtmcu_bql_locked())` when calling them to prove thread safety at runtime.
- **Assumption: Lifecycles are clean.** We assume `instance_finalize` correctly `drop`s the `Box::into_raw` state. But there are no tests for hot-unplugging devices to verify memory is reclaimed.
- **Assumption: Endianness is handled.** We assume MMIO ops correctly use `DEVICE_LITTLE_ENDIAN`, but we do not enforce that our Rust struct byte-layouts or manual shifts are verified against simulated guest behavior under stress.

## 3. What should be done better?
- **Restore `zenoh-802154`**: We must backport the 669-line MAC layer logic from before the Phase 19 migration into the pure-Rust QOM structure.
- **Add runtime BQL locking**: All Zenoh subscriber callbacks *must* wrap QEMU state changes in `virtmcu_bql_lock() / virtmcu_bql_unlock()`. (Applied to `zenoh-ui` and `zenoh-chardev`).
- **Improve Smoke Tests**: Phase 14 (`zenoh-802154`) needs a real runtime execution test, not just a CLI parse check.
- **Stress Tests**: We need a multithreaded test script that hammers `zenoh-chardev` and `zenoh-ui` from an external Zenoh node while the guest is running, ensuring no segfaults occur due to missing BQL locks.
- **Coverage**: We must run `make test-coverage-guest` or `cargo llvm-cov` on `virtmcu-qom` to ensure the new macros (`declare_device_type!`, `define_properties!`) and FFI layers are fully tested.
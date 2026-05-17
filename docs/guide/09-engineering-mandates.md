# The VirtMCU Engineering Mandates

## The Enterprise Quality Mandate

To maintain the status of VirtMCU as the definitive, high-performance, deterministic simulation framework, every line of code must adhere to the **Enterprise Quality Mandate**. We do not take "AI-style" shortcuts. We do not suppress lints. We do not bypass the type system. Every change must be surgically precise, idiomatically perfect, and backed by empirical test evidence.

This chapter serves as the immutable law for all developers—human or agent—contributing to the VirtMCU ecosystem.

---

## 1. The Core Constants

### Binary Fidelity (RFC-0006)
**The same firmware ELF that runs on a real MCU must run unmodified in VirtMCU.**
- No virtmcu-specific startup code or linker sections.
- Peripherals mapped at **exact** datasheet base addresses.
- Infrastructure (clocks, co-sim) must be **invisible** to the guest firmware.
- Any feature requiring firmware modification is a VirtMCU bug.

### Global Simulation Determinism (RFC-0001)
**Same topology + same firmware + same `global_seed` → bit-identical output.**
- **Topology declared, not discovered**: Runtime Zenoh "scouting" is BANNED.
- **Canonical tie-breaking**: Messages delivered in order `(vtime, node_id, seq)`.
- **Stochastic seeding**: Derive per-node PRNG as `seed_for_quantum(global_seed, node_id, quantum_number)`. `rand::thread_rng()` is BANNED.

---

## 2. Production Engineering Standards

### Environment Agnosticism
- **No absolute paths**: All paths must be relative to the project root.
- **Cross-platform path handling**: Use `PathBuf` (Rust) or `std::filesystem` (C++).
- **Devcontainer-first**: `localhost` is the container. Never assume host toolchain access.

### Explicit Constants
- **No Magic Numbers**: BANNED: inline literals. REQUIRED: named `const` with a comment explaining the value and purpose.

### Logging Strictness
- **No `print()` / `println!`**: BANNED in production code. 
- **Structured Logging**: Use `sim_info!/sim_err!` (Rust) and `tracing::info/error` (Rust tools).

### Protocol Serialization
- **No Manual Struct Packing**: BANNED: manual packing/unpacking of bytes.
- **Schema-First**: REQUIRED: Use `virtmcu-wire` (FlatBuffers) for all core simulation protocols (see RFC-0012).

### No Polling / Sleep Avoidance
- **BANNED**: `std::thread::sleep`, `tokio::time::sleep`, or `time.sleep()` in hot paths, MMIO, or tests.
- **Deterministic Sync**: Use `vta.step()`, QMP events, or Zenoh `recv_async()`. 
- **Exception**: `// virtmcu-allow: sleep reasoning="..."` is required for the few unavoidable cases.
---

## 3. The "Beyoncé Rule" of Verification
> "If you liked it, then you shoulda put a test on it."

- **Empirical Reproduction**: You must write a failing test reproducing a bug **before** applying the fix.
- **Coverage**: Every feature must be backed by unit or integration tests (Rust).
- **Stress Testing**: New features must survive 10,000+ iterations (`cargo test --release`) or 100+ integration runs (see RFC-0040).

---

## 4. Concurrency & Safety Mandates

### Safe Big QEMU Lock (BQL) Usage
- **Async threads**: MUST NOT block waiting for BQL. Use `crossbeam_channel` to drain into a QEMU timer. `VtimeIngress` (RFC-0021) handles this pattern automatically for ingress.
- **MMIO vCPU threads**: Yield BQL via `Bql::temporary_unlock()` when blocking on external I/O (see RFC-0018).
- **Bql API**: Use the RAII `Bql::lock()` and `QemuCond::wait_yielding_bql`.
- **Lock Order**: BQL → peripheral mutex → condvar wait.

### Two-Stage Delivery Pipeline
- **Never mutate guest-visible state or wake a suspended vCPU directly inside a transport callback.**
- **Stage 1 (Host Ingress)**: Use `VtimeIngress` to queue payloads by `delivery_vtime_ns` in a virtual-time-sorted priority queue.
- **Stage 2 (Virtual Time Delivery)**: The `VtimeIngress` schedules a `QomTimer` (bound to `QEMU_CLOCK_VIRTUAL`) that fires at `delivery_vtime_ns`. The delivery callback performs the register mutation or signals IRQs under the BQL.

### Safe Peripheral Teardown
- **No Bounded Spinloops**: BANNED: `while attempts < N`. This leads to time-bomb Use-After-Free (UAF) bugs.
- **The Drain Pattern**: Use `Condvar::notify_all()` + unconditional `Condvar::wait()` in the `Drop` implementation to ensure all vCPUs have exited the MMIO path before the device is freed.

---

## 5. Language-Specific Mandates

### Rust: The Memory Safety Wall
- **Packed Structs**: Use `ptr::read_unaligned`.
- **Endianness**: Use `to_le_bytes()`. `to_ne_bytes()` is BANNED for wire data.
- **Unsafe Scope**: One FFI call per `unsafe` block.

---

## 6. Common Anti-Patterns (The "Wall of Shame")

1.  **Hardcoded Ports**: Never use `7447` or `7450`. Use dynamic port generation.
2.  **Hardcoded Paths**: Never use `/tmp/out.dtb`. Use `virtmcu-test-runner` `tmp_path`.
3.  **Manual Process Management**: Never spawn daemons in test bodies. Use `virtmcu-test-runner` fixtures.
4.  **Stale Processes**: Always run `make clean-sim` if a test fails; orphaned QEMUs hold ports.
5.  **DSO TLS Trap**: Never call QEMU TLS macros (like `bql_locked()`) from a plugin DSO. Peripheral code never needs to query BQL status at all — the framework passes `&BqlContext` as compile-time proof. Only `virtmcu-qom` framework internals may use `Bql::is_held()` (which wraps `virtmcu_is_bql_locked()` from the main binary export). If you think you need to check BQL status in peripheral code, you are writing framework code and it belongs in `virtmcu-qom`.

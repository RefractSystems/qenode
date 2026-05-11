# The Temporal Core

## Learning Objectives
After this chapter, you can:
1. Explain the three clock modes of VirtMCU and their use cases.
2. Describe the request/reply protocol for clock synchronization.
3. Understand how the BQL is handled during virtual time pauses.

## The Philosophy of Time: Physics as the Master

In a standard emulator, time is an afterthought. The emulator typically runs as fast as possible, using the host's wall-clock to drive its internal timers. In the VirtMCU digital twin ecosystem, this is unacceptable. The firmware in the **Cyber Node** interacts with a physical world (simulated by the **Physical Node**, such as a drone in MuJoCo) governed by continuous differential equations. If the Cyber Node runs free, the firmware's control loops will desynchronize from the physics.

**The Golden Rule of VirtMCU**: The Physical Node (acting as the TimeAuthority) owns the clock. Virtual time inside the Cyber Node only advances when the external authority explicitly grants a "quantum" of time over the Transport Layer.

---

## 1. The Three Modes of Time

VirtMCU (our QEMU-based Cyber Node implementation) provides three distinct clock modes to balance simulation accuracy with host performance.

| Mode | How to Invoke | Accuracy | Throughput | Use Case |
|---|---|---|---|---|
| **Standalone** | *(omit `-device virtmcu-clock`)* | Wall-clock | 100% | Pure firmware unit testing; no physics engine. |
| **Slaved-Suspend** | `-device virtmcu-clock,mode=slaved-suspend` | Quantum-accurate | ~95% | **Default.** Control loops ≥ 1ms. TB-boundary pauses. |
| **Slaved-Icount** | `-device virtmcu-clock,mode=slaved-icount` | Instruction-accurate | ~15–20% | PWM, µs-precision DMA. QEMU uses `-icount shift=0`, guaranteeing 1 instruction = 1 virtual ns. |
| **Slaved-Unix** | `-device virtmcu-clock,mode=slaved-unix` | Quantum-accurate | ~98% | High-performance local co-simulation via Unix Sockets. |

---

## 1.1 Clock Transport Modes

The `mode` parameter also determines which transport layer is used to communicate with the `TimeAuthority`.

| `mode` parameter   | Transport                    | `sim/clock/start` needed? |
|--------------------|------------------------------|---------------------------|
| `standalone`       | None (QEMU free-runs)        | No                        |
| `slaved-unix`      | `UnixSocketClockTransport`   | **No** — exits via `clock_init_with_transport()`, Zenoh code never runs |
| `slaved-suspend`   | `ZenohClockTransport`        | Only if `is_coordinated=true` |
| `slaved-icount`    | `ZenohClockTransport`        | Only if `is_coordinated=true` |

The `sim/clock/start` Zenoh topic is only relevant when `is_coordinated=true` AND using Zenoh transport. It is not subscribed to and has no effect in `slaved-unix` mode.

---

## 2. The Wire Protocol (Formal Specification)

The `clock` device communicates with the `TimeAuthority` via the **Control Plane**. This is a strictly 1:1, low-latency RPC channel.

### Request: TimeAuthority → Node
**Topic**: `sim/clock/advance/{node_id}`
**Payload** (24-byte FlatBuffer struct):
- `delta_ns` (uint64): The size of the quantum to execute in virtual nanoseconds.
- `absolute_vtime_ns` (uint64): The current absolute time in the physics world.
- `quantum_number` (uint64): Global sequence number for the current quantum.

### Reply: Node → TimeAuthority
**Payload** (24-byte FlatBuffer struct):
- `current_vtime_ns` (uint64): The actual virtual time reached by QEMU.
- `n_frames` (uint32): Count of pending Ethernet frames (informational).
- `error_code` (uint32):
    - **`0 (OK)`**: Success. Quantum completed.
    - **`1 (STALL)`**: STALL DETECTED. QEMU failed to reach the TB boundary within the wall-clock timeout. QEMU stays alive for debugging.
    - **`2 (ZENOH_ERROR)`**: Transport layer or protocol failure.
- `quantum_number` (uint64): The sequence number of the quantum being acknowledged.

### The Stall-Timeout Contract
To prevent deadlocks in CI, every quantum has a wall-clock `stall-timeout`. 
- **Dynamic Scaling**: Timeouts are mathematically stretched based on the environment (e.g., 5.0x multiplier under ASan).
- **Logical Timeouts**: Developers pass ideal *logical* timeouts to the test harness; the framework handles the real-world mapping transparently.
- **NEVER hardcode** `stall-timeout` in world YAMLs.

---

## 3. The Mechanism: TCG Hooks and the BQL

To achieve deterministic pauses, VirtMCU hooks into the heart of the QEMU execution loop.

### The TCG Quantum Hook
We inject a function pointer into `accel/tcg/cpu-exec.c`. At the end of every Translation Block (TB), QEMU calls the VirtMCU hook. If the requested quantum has expired, the hook pauses the vCPU and waits for the next command.

### The BQL "Unlock-and-Park" Pattern
QEMU uses the **Big QEMU Lock (BQL)** to protect hardware state. VirtMCU uses a safe RAII pattern to avoid deadlocks:
1.  **Detect** quantum expiry.
2.  **Signal** the background thread that the quantum is done.
3.  **Wait** on a condition variable using `virtmcu_qom::sync::Condvar::wait_yielding_bql`. 

This pattern internally uses `Bql::temporary_unlock()` to safely yield the lock, allowing GDB or QMP to inspect the guest while it is paused.

---

## 4. Virtual Time in Practice

### WFI (Wait For Interrupt)
When a guest executes `WFI`, the vCPU stops. 
- In **Slaved-Suspend**, virtual time still advances during WFI. Quantum boundaries still trigger clock-halts. 
- **Optimization**: Prefer the ARM Generic Timer at 100 Hz over tight polling to minimize host CPU wakeups.

### MMIO Socket Blocking
The `mmio-socket-bridge` blocks the QEMU TCG thread for every MMIO operation. 
- **Zero-Time Transactions**: From the firmware's perspective, the external transaction takes 0 virtual nanoseconds.
- **Latency Sensitivity**: High latency in the external socket leads to clock stalls. Virtual time does **NOT** advance while blocked.

### Test Automation
Our Python test harness uses **Virtual Time Timeouts**. A test saying `await bridge.wait_for_line("Boot Complete", timeout=5.0)` is waiting for 5 *virtual* seconds, making it immune to host slowdowns.

---

## See Also
*   **[PDES and Virtual Time](../fundamentals/08-pdes-and-virtual-time.md)**: The theoretical foundation of clock synchronization.
*   **[BQL and Concurrency](../fundamentals/10-bql-and-concurrency.md)**: Deep dive into the locking mechanisms described in Section 3.
*   **[Debugging Playbook](../guide/07-debugging-playbook.md)**: Troubleshooting "Stall Detected" errors.

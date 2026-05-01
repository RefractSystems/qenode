# Chapter 2: The Temporal Core

## The Philosophy of Time: Physics as the Master

In a standard emulator, time is an afterthought. QEMU typically runs as fast as possible, using the host's wall-clock to drive its internal timers. In the VirtMCU digital twin ecosystem, this is unacceptable. The firmware interacts with a physical world (e.g., a drone in MuJoCo) governed by continuous differential equations. If QEMU runs free, the firmware's control loops will desynchronize from the physics.

**The Golden Rule of VirtMCU**: The Physics Engine (TimeAuthority) owns the clock. Virtual time only advances when the external authority explicitly grants a "quantum" of time.

---

## 1. The Three Modes of Time

VirtMCU provides three distinct clock modes to balance simulation accuracy with host performance.

| Mode | QEMU Arguments | Accuracy | Throughput | Use Case |
|---|---|---|---|---|
| **Standalone** | *(omit `-device clock`)* | Wall-clock | 100% | Pure firmware unit testing; no physics engine. |
| **Slaved-Suspend** | `-device clock` | Quantum-accurate | ~95% | **Default.** Control loops ≥ 1ms. TB-boundary pauses. |
| **Slaved-Icount** | `-device clock,mode=icount` | Instruction-accurate | ~15–20% | PWM, bit-banging, µs-precision DMA. QEMU uses `-icount shift=0`, guaranteeing an exact 1 instruction = 1 virtual nanosecond relationship. |

---

## 2. The Wire Protocol (Formal Specification)

The `clock` device communicates with the `TimeAuthority` via the **Control Plane**. This is a strictly 1:1, low-latency RPC channel.

### Request: TimeAuthority → Node
**Topic**: `sim/clock/advance/{node_id}`
**Payload** (16-byte FlatBuffer struct):
- `delta_ns` (uint64): The size of the quantum to execute in virtual nanoseconds.
- `mujoco_time_ns` (uint64): The current absolute time in the physics world.

### Reply: Node → TimeAuthority
**Payload** (16-byte FlatBuffer struct):
- `current_vtime_ns` (uint64): The actual virtual time reached by QEMU.
- `n_frames` (uint32): Count of pending Ethernet frames (informational).
- `error_code` (uint32):
    - `0 (OK)`: Success.
    - `1 (STALL)`: QEMU failed to reach the boundary within the wall-clock timeout.
    - `2 (ERROR)`: Transport or protocol failure.

---

## 3. The Mechanism: TCG Hooks and the BQL

To achieve deterministic pauses, VirtMCU hooks into the heart of the QEMU execution loop.

### The TCG Quantum Hook
We inject a function pointer into `accel/tcg/cpu-exec.c`. At the end of every Translation Block (TB), QEMU calls the VirtMCU hook. If the requested quantum has expired, the hook pauses the vCPU and waits for the next command.

### The BQL "Unlock-and-Park" Pattern
QEMU uses the **Big QEMU Lock (BQL)** to protect hardware state. If we block the vCPU thread while holding the BQL, the entire process (including QMP and GDB) deadlocks. VirtMCU uses a safe RAII pattern:
1.  **Detect** quantum expiry.
2.  **Signal** the background thread that the quantum is done.
3.  **Wait** on a condition variable using `virtmcu_qom::sync::Condvar::wait_yielding_bql`. This internally uses `Bql::temporary_unlock()` to safely yield the lock and automatically re-acquires it before resuming execution.

This ensures the emulator remains responsive even while "paused" in virtual time.

---

## 4. Virtual Time in Practice

### WFI (Wait For Interrupt)
When a guest executes `WFI`, the vCPU stops. 
- In **Slaved-Icount** mode, virtual time "warps" forward to the next pending timer deadline.
- If no timers are pending, time remains frozen until an external interrupt (e.g., a packet from the coordinator) wakes the CPU.

### MMIO Socket Blocking
When a peripheral access blocks on an external socket (e.g., SystemC), virtual time **does not advance**. From the firmware's perspective, the external transaction takes 0 nanoseconds, regardless of host-side latency. This maintains perfect causal ordering between the CPU and external hardware models.

### Test Automation
Our Python test harness (`qmp_bridge.py`) tracks virtual time by polling the emulator's `icount`. This allows tests to use **Virtual Time Timeouts**. A test can say "wait for this UART string, but fail if it doesn't appear within 5 *virtual* seconds," ensuring the test is immune to host CPU load or ASan-induced slowdowns.

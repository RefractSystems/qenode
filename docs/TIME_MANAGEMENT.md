# virtmcu Time Management and Synchronization

This document explains how **virtmcu** handles time. It covers how QEMU is forced to act as a deterministic "time slave" to an external physics engine, the different clock modes available, the internal mechanisms (TCG hooks and the Big QEMU Lock), and the rationale behind these design choices.

---

## 1. The Core Philosophy: Physics is the Master

Standard QEMU is designed to run as fast as possible, using the host computer's wall-clock time to drive its internal virtual timers. 

In a Cyber-Physical System (CPS) digital twin, this is unacceptable. The firmware must interact with a simulated physical world (e.g., a drone in MuJoCo). If QEMU runs free, the drone's control loop will execute thousands of times before the physics engine calculates a single frame of gravity.

**Rule #1 of virtmcu: The Physics Engine (TimeAuthority) owns the clock.**

QEMU's virtual time does not advance on its own. It only advances when the external TimeAuthority explicitly commands it to, via a native Zenoh network message.

### The Architecture

```mermaid
sequenceDiagram
    participant TA as TimeAuthority (Physics)
    participant ZC as hw/zenoh/zenoh-clock.c (QEMU)
    participant TCG as QEMU CPU Loop (TCG)
    participant FW as Guest Firmware

    Note over TA,FW: The Physics Engine defines a quantum (e.g., 1ms)
    
    TA->>ZC: Zenoh GET: sim/clock/advance/0 (delta_ns=1,000,000)
    Note over ZC: QEMU resumes execution
    ZC->>TCG: Release lock, allow execution
    TCG->>FW: Execute ARM/RISC-V instructions
    Note over FW: Firmware runs its control loop
    FW->>TCG: (1ms of virtual time elapses)
    TCG->>ZC: TCG Hook: Quantum Complete!
    Note over ZC: QEMU pauses execution
    ZC-->>TA: Zenoh REPLY: current_vtime_ns
    Note over TA: Physics engine calculates gravity, collisions, etc.
```

---

## 2. The Three Modes of Time

virtmcu provides three distinct clock modes, controlled by whether the `zenoh-clock` device is instantiated and which properties are passed to it.

| Mode | QEMU Arguments | Speed | When to use |
|---|---|---|---|
| **Standalone** | *(omit `-device zenoh-clock`)* | 100% | Pure firmware development, CI/CD unit tests where physics are not required. QEMU uses host wall-clock time. |
| **Slaved-Suspend** | `-device zenoh-clock,node=N,router=<url>` | ~95% | **Default for FirmwareStudio.** QEMU runs at full native speed but pauses at Translation Block boundaries when the physics quantum expires. Ideal for control loops >= one quantum. |
| **Slaved-icount** | `-device zenoh-clock,node=N,router=<url>,mode=icount`<br>`-icount shift=0,align=off,sleep=off` | ~15–20% | Strict instruction-counting mode. 1 instruction = 1 virtual nanosecond. Required when firmware measures sub-quantum intervals (PWM, bit-banging, microsecond DMA). |

Where `N` is the QEMU node index (0-based) and `<url>` is the Zenoh router address (e.g. `tcp/127.0.0.1:7447`).

---

## 3. The Mechanism: TCG Hooks and the BQL

QEMU's emulator is essentially a giant `while(true)` loop that translates guest instructions into host instructions (Tiny Code Generator, or TCG) and executes them in "Translation Blocks" (TBs).

Out of the box, QEMU does not have a plugin API to say "Pause execution after exactly *X* nanoseconds and wait for a network packet."

### The TCG Quantum Hook
We use a python script (`patches/apply_zenoh_hook.py`) to inject a function pointer directly into `accel/tcg/cpu-exec.c`. At the end of every Translation Block, QEMU calls `virtmcu_tcg_quantum_hook()`.

### The Big QEMU Lock (BQL) Dance
QEMU is notoriously heavily threaded, yet relies on a single massive mutex called the **Big QEMU Lock (BQL)** to protect hardware state. 

If our `zenoh-clock` receives a network packet and tries to pause the CPU while holding the BQL, the entire QEMU process deadlocks. The QMP monitor, GDB stub, and UART output all freeze because they are waiting for the BQL to process I/O.

Here is how `hw/zenoh/zenoh-clock.c` safely handles the suspend/resume cycle:

```mermaid
flowchart TD
    Start((TCG Hook Triggered)) --> CheckTime{Has quantum expired?}
    CheckTime -- No --> Continue((Return to TCG))
    CheckTime -- Yes --> SetDone[Set quantum_done = true]
    SetDone --> SignalQuery[Signal query_cond]
    SignalQuery --> BQLUnlock[bql_unlock()]
    
    BQLUnlock --> WaitVCPU[qemu_cond_wait(&vcpu_cond)]
    
    subgraph Zenoh Network Thread
        WaitQuery[Wait for sim/clock/advance] --> WakeUp[Wake up]
        WakeUp --> WaitDone[Wait for quantum_done]
        WaitDone --> Reply[Send Zenoh Reply to Physics]
        Reply --> ReadNewDelta[Read next delta_ns]
        ReadNewDelta --> SignalVCPU[Signal vcpu_cond]
    end
    
    WaitVCPU -. Woken by Zenoh .-> BQLLock[bql_lock()]
    BQLLock --> UpdateTimers[Update virtual timers]
    UpdateTimers --> Continue
```

By explicitly releasing the BQL (`bql_unlock()`) *before* we block the CPU thread waiting for the next physics tick, the rest of QEMU (networking, QMP, serial ports) remains alive and responsive.

---

## 4. Virtual Time in Test Automation

In automated testing (CI/CD), Robot Framework test scripts normally use `time.sleep()` or wall-clock timeouts to wait for UART output. 

When running in `slaved-icount` mode, QEMU is heavily throttled. A firmware routine that takes 2 seconds in real life might take 15 seconds of wall-clock time to simulate. Wall-clock timeouts will cause false-positive test failures.

To solve this, our Python testing library (`qmp_bridge.py`) tracks **virtual time**:
1. It polls QEMU via the QEMU Machine Protocol (QMP) using the `query-replay` command.
2. `query-replay` returns the current instruction count (`icount`), which in `shift=0` mode exactly correlates to virtual nanoseconds.
3. Tests timeout based on *simulated* seconds, guaranteeing rock-solid stability regardless of host CPU load or simulation speed.

For the implementation details and worked examples, see
[tutorial/lesson11.2-virtual-time-timeouts/README.md](../tutorial/lesson11.2-virtual-time-timeouts/README.md).

---

## 5. Design Rationale: Why do it this way?

During the design of virtmcu, several alternative approaches were evaluated. Here is why the current architecture was chosen.

### Why not use SystemC to embed QEMU? (The MINRES approach)
Frameworks like MINRES `libqemu-cxx` compile QEMU as a C++ library and wrap it in a SystemC TLM-2.0 module. 
* **The Problem:** It is incredibly invasive. It forces the entire simulation environment to be written in C++ and SystemC, making it difficult to integrate with modern Python-based AI workflows or external physics engines like MuJoCo or OpenUSD.
* **Our Solution:** Zenoh. We get TLM-2.0 equivalent transaction-level modeling over a blazingly fast network bus. QEMU remains a standalone process, and any language (Python, Rust, C++) can coordinate time with it.

### Why not just use `-icount` with a Python script adjusting `qemu_icount_bias`?
Earlier prototypes of FirmwareStudio tried to run a Python agent that paused QEMU, adjusted the icount bias via QMP, and resumed it.
* **The Problem:** Python in the hot simulation loop is disastrous for performance. Pausing QEMU via QMP took ~2 milliseconds per quantum. If the quantum is 1ms, the simulation runs at 0.5x real-time purely due to IPC overhead.
* **Our Solution:** `hw/zenoh/zenoh-clock.c` is compiled natively into QEMU. The Zenoh Rust/C backend handles the network synchronization in microseconds.

### Why have `slaved-suspend` if `slaved-icount` is more accurate?
* **The Problem:** `-icount` forces QEMU to count every single instruction, disabling many of TCG's speed optimizations. It cuts performance by ~80%.
* **Our Solution:** 95% of firmware does not care about sub-nanosecond instruction timing; it only cares that the UART fires or the physics state updates every millisecond. `slaved-suspend` lets TCG run at full native speed, only intercepting the loop at the very end of the millisecond to wait for physics. We keep `slaved-icount` strictly for when cycle-accurate PWM or DMA timing is required.

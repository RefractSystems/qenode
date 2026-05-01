# Postmortem: SystemC Warning W571 (No Activity) and TSAN Data Races

## Overview
During the execution of the irq_stress smoke tests and ASan/TSan CI pipelines, the SystemC adapter generated massive amounts of console spam with the following warning:
`Warning: (W571) no activity or clock movement for sc_start() invocation`

Simultaneously, ThreadSanitizer (TSAN) flagged data races during the adapter's teardown sequence.

## What is W571?
The SystemC Warning (W571) indicates that `sc_start()` was called, but no runnable processes, delta notifications, or time-outs occurred, resulting in no simulation time progression or activity. It usually means the simulation is "starved" of events.

### Common Causes and Solutions (General SystemC)
*   **Clock Not Running**: Ensure `sc_clock` is properly instantiated and connected. If the clock is not toggling, no time advances.
*   **No Active Processes**: Check that `SC_METHOD`, `SC_THREAD`, or `SC_CTHREAD` are correctly declared and sensitive to events that actually occur.
*   **Simulation Finished Early**: If `sc_start()` is called after all processes have already terminated or hit a `wait()` that never wakes up.
*   **Incorrect Port Binding**: Check if signals are connected correctly. Unbound ports may lead to no activity in sensitive methods.

### General Troubleshooting
*   Verify the `sc_start()` argument. If using `sc_start(0)` or `sc_start(SC_ZERO_TIME)`, this warning can be normal, though still indicating a lack of pending events.
*   Use `sc_pending_activity_at_future_time()` to check if there are any events scheduled later.
*   Ensure your `sc_clock` is not constrained by a `stop_time` too early.

## VirtMCU Specific Root Cause
In the `tools/systemc_adapter/main.cpp`, the OS main thread contained a busy-wait polling loop:
```cpp
while (running) {
  sc_start();
  std::this_thread::sleep_for(std::chrono::microseconds(100));
}
```
Because VirtMCU bridges QEMU and SystemC via an external UNIX socket, there are legitimate periods where the SystemC kernel has absolutely no internal pending events while it waits for a QEMU MMIO request. Calling `sc_start()` continuously in this state triggered the `W571` warning continuously.

Furthermore, the `running` flags used to shut down the `QemuAdapter` and `SharedMedium` threads were declared as plain `bool` types. Setting `running = false` from the main thread while the IO thread was reading it caused a classic Data Race, tripping TSAN in CI.

## Resolution
1.  **Reactive Architecture (No Polling)**: The 100µs busy-wait was completely removed. SystemC is now only ticked if `sc_pending_activity()` is true. Otherwise, the main OS thread sleeps natively using a `std::condition_variable` (`KernelWakeup`) until the UNIX socket thread injects a new packet and calls `g_kernel.wake()`.
2.  **Warning Suppression**: Because event-driven co-simulation inherently involves invoking `sc_start(SC_ZERO_TIME)` to process cross-thread updates (`async_request_update()`) even when no explicit SystemC delta cycle is advancing, we explicitly suppress this warning to prevent CI log bloat:
    ```cpp
    sc_report_handler::set_actions("/OSCI/SystemC/kernel/sc_start/no_activity", SC_DO_NOTHING);
    ```
3.  **Thread Safety**: Promoted all `running` flags to `std::atomic<bool>`.
4.  **Socket Validation**: Replaced blocking I/O with `poll()` bounded reads/writes to ensure teardown condition variables are never permanently blocked if QEMU crashes mid-transaction.

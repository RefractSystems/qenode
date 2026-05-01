# Postmortem: QEMU Plugin Dynamic Loading and Visibility

## Date
2026-04-29

## The Incident
During the integration of Telemetry, IEEE802.15.4, and the Zenoh Coordinator stress testing, we observed spontaneous failures of the `test_telemetry` and clock-related tests. The QEMU emulator was failing to load the external QOM plugins (e.g., `hw-virtmcu-telemetry.so` and `hw-virtmcu-clock.so`), terminating with:

```text
QEMU Plugin Load Error: undefined symbol: virtmcu_cpu_set_tcg_hook
```

## Root Cause
To facilitate deterministic synchronization and observation, VirtMCU injects global function pointers (hooks) into the core QEMU execution loop (e.g., `accel/tcg/cpu-exec.c` and `hw/core/irq.c`). 

Initially, these function pointers were declared simply as:
```c
void virtmcu_cpu_set_tcg_hook(void (*cb)(CPUState *cpu)) { ... }
```

However, QEMU is typically compiled with `-fvisibility=hidden`. This means that by default, symbols defined in the main executable (`qemu-system-arm`) are **not exported** to the dynamic symbol table (`.dynsym`). When the QOM plugins (`.so` files) were loaded at runtime via `dlopen()`, the dynamic linker could not resolve the references to these setters, causing immediate module load failure.

## The Resolution
1. **Explicit Symbol Export:** We updated the `patches/apply_zenoh_hook.py` script to enforce default visibility on all globally injected `virtmcu_*_hook` setters:
   ```c
   __attribute__((visibility("default"))) void virtmcu_cpu_set_tcg_hook(void (*cb)(CPUState *cpu)) { ... }
   ```
2. **Proper QOM Hooks Implementation:** We updated the Rust QOM plugins (`hw/rust/observability/telemetry` and `hw/rust/backbone/clock`) to rely entirely on these exported setters (e.g., `virtmcu_qom::cpu::virtmcu_cpu_set_halt_hook`) rather than trying to assign values across DSO boundaries to raw `static mut` pointers, which relies on unpredictable relocation behaviors across shared objects.

## Preventative Measures (SOTA)
To ensure this issue never reoccurs or bites developers attempting to add new hooks in the future:
1. **Automated Verification:** The `scripts/verify-exports.py` script has been extended. It now actively inspects the final `qemu-system-arm` executable using `nm -D` and asserts that all `virtmcu_` hook functions are genuinely present in the dynamic symbol table as exported `T` (text) or `B` (bss) symbols.
2. **CI Enforcement:** Because `scripts/verify-exports.py` runs as part of the `make build-tools` target, any failure to correctly mark a new hook with `visibility("default")` will result in a hard build break before tests even begin.

# ADR-006: Binary Fidelity — Same ELF on Real MCU and VirtMCU

## Status
Accepted

## Context

A digital twin is only useful if the virtual MCU runs the same software as the physical
one. If firmware must be recompiled, patched, or conditionally compiled to work inside
the simulator, engineers will maintain two codebases — one for hardware, one for the sim
— and the simulator's test results will not be trustworthy evidence about real-hardware
behavior.

This concern is concrete, not theoretical. Renode requires a `.repl` that maps devices
at Renode's canonical addresses, which often differ from the physical MCU's datasheet
addresses. Engineers working with Renode frequently discover that the firmware built for
the sim crashes on silicon because the peripheral base addresses or interrupt numbers
differ. virtmcu must not repeat this mistake.

## Decision

**The same ELF binary that is flashed to a physical MCU must boot and execute correctly
inside VirtMCU without any modifications to the firmware source or build system.**

### Enforcement rules

1. **Address fidelity**: Every peripheral in the YAML platform description must be
   placed at the base address specified in the target MCU's datasheet. `yaml2qemu.py`
   validation must catch address mismatches against the reference DTB.

2. **Register layout fidelity**: QOM peripheral models must implement the exact register
   map of the silicon they model (same offsets, same reset values, same read/write masks).
   Unimplemented registers must return the reset value (not zero, if those differ) and
   must not fault unless the real hardware would fault.

3. **Interrupt fidelity**: IRQ numbers in the DTB `interrupts` property must match the
   NVIC/GIC numbers in the datasheet. virtmcu co-simulation devices (`zenoh-clock`,
   `zenoh-netdev`) must not consume interrupt lines that the target MCU exposes to
   firmware.

4. **Co-simulation transparency**: `zenoh-clock`, `zenoh-netdev`, and `zenoh-chardev`
   are QEMU-level devices only. They must not appear as MMIO regions in the guest
   physical address space. Adding `-device zenoh-clock` must be equivalent to adding an
   oscilloscope probe — it observes and controls timing without altering the circuit the
   firmware sees.

5. **No firmware API**: virtmcu must not define a guest-visible semihosting extension,
   mailbox, or special memory region that firmware calls to interact with the simulator.
   Any such interface would break binary fidelity immediately.

6. **Standalone mode must be transparent**: Running without `-device zenoh-clock`
   (standalone mode) must produce functionally identical firmware behavior to running
   with it. The only permitted difference is wall-clock speed and virtual-time
   determinism.

### What binary fidelity does NOT require

- Cycle-accurate timing: VirtMCU uses TCG, not RTL simulation. Tight busy-wait loops
  that depend on exact cycle counts (e.g., bitbanged SPI delays) may behave differently.
  This is documented and expected. Firmware that relies on cycle-exact timing for
  correctness (not just performance) is considered a firmware defect.
- On-chip debug (SWD/JTAG): VirtMCU exposes GDB stub, which is functionally equivalent
  for all debugging purposes firmware is aware of.
- Exact reset behavior for undocumented silicon errata: If the datasheet does not
  specify a behavior, VirtMCU is not required to replicate undocumented silicon quirks.

## Consequences

### Positive
- Engineers flash the same binary to hardware and sim. If the sim passes, hardware has
  a high probability of passing too.
- No "sim-only" firmware branches to maintain.
- VirtMCU peripheral models serve as executable specifications of the real hardware
  register maps — reviewable, diffable, testable.

### Negative
- Peripheral model quality becomes load-bearing. A wrong reset value in a QOM model
  will produce a binary-compatible crash (firmware aborts the same way it would on
  broken silicon) but the root cause is harder to diagnose.
- Matching the exact address map of every supported MCU requires per-target YAML
  platform files. We cannot use QEMU's generic `virt` machine addresses for real
  firmware targets.

## Test Requirements

Binary fidelity must be verified by a dedicated test suite separate from unit tests:

1. **Golden binary test** (`tests/test_binary_fidelity.robot`): For each supported
   target MCU, maintain a reference firmware binary that has been validated to produce
   known UART output on real silicon. The test boots this binary in VirtMCU and asserts
   the same output. The binary is checked into `tests/firmware/` as a pre-built ELF
   with a corresponding `SHA256SUMS` file.

2. **Peripheral register smoke test**: A firmware that reads back every implemented
   peripheral register after reset and prints the values over UART. Expected output is
   captured from real silicon and stored as a golden file. Any deviation flags a
   register-map regression in VirtMCU.

3. **No-firmware-change CI gate**: The CI pipeline must include a step that builds the
   reference firmware with a standard ARM cross-compiler (no virtmcu flags) and runs
   the golden binary test. If the test requires a firmware rebuild, the build step is
   considered a fidelity failure.

## Golden Firmware References

Golden reference firmware binaries used for validation are stored in `tests/firmware/`. 
These files are managed via Git LFS and represent the ground truth for hardware emulation.
Currently, this includes:
- `cortex-a15-virt/echo.elf`: Basic UART echo firmware for ARM Cortex-A15.

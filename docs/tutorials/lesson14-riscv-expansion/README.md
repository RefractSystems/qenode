# Lesson 11: RISC-V Expansion & Cross-Architecture Simulation

In this lesson, we will explore how **virtmcu** handles multiple CPU architectures beyond ARM. We will build a minimal RISC-V firmware, define a RISC-V platform using a Device Tree, and run it using our unified `run.sh` wrapper.

## Prerequisites

- RISC-V cross-compiler: `gcc-riscv64-linux-gnu`
- Device Tree Compiler: `dtc`
- virtmcu QEMU built with RISC-V targets (run `scripts/setup-qemu.sh` to ensure this)

## Architecture Agnosticism in virtmcu

One of the key goals of virtmcu is to provide a unified interface for hardware simulation, regardless of the target CPU. While ARM support was our initial focus due to the `arm-generic-fdt` machine, we have extended the framework to support RISC-V by leveraging QEMU's highly flexible `virt` machine for RISC-V.

The `run.sh` script automatically detects the target architecture from the hardware description (REPL, YAML, or DTS) and selects the appropriate QEMU binary (`qemu-system-riscv64` or `qemu-system-arm`).

## Building the RISC-V Firmware

We've provided a minimal "Hello World" assembly firmware for RISC-V in `tests/fixtures/guest_apps/boot_riscv/hello.S`. It interacts with the NS16550 UART (standard on the RISC-V `virt` machine) located at `0x10000000`.

```bash
# Build the firmware and the Device Tree
make -C tests/fixtures/guest_apps/boot_riscv
```

## Running the Simulation

You can run the RISC-V simulation using the same `run.sh` script used for ARM:

```bash
./scripts/run.sh --dts tests/fixtures/guest_apps/boot_riscv/minimal.dts --kernel tests/fixtures/guest_apps/boot_riscv/hello.elf -nographic
```

You should see the output:
```
HI RV
```

## How it works

1.  **Architecture Detection**: `run.sh` parses the input file. If it sees `RISCV` in a REPL/YAML or if explicitly told via `--arch riscv`, it switches to the RISC-V toolchain.
2.  **Machine Selection**: For RISC-V, it uses the `-M virt` machine, which supports loading an external DTB via `-dtb`.
3.  **FDT Compatibility**: Our `FdtEmitter` generates a Device Tree compatible with the RISC-V `virt` machine's expectations (e.g., correct CPU nodes, interrupt controllers, and peripheral mappings).

## Summary

By abstracting the architecture-specific details into our tooling (`repl2qemu`, `yaml2qemu`, and `run.sh`), we enable developers to focus on firmware logic and hardware connectivity without worrying about the underlying emulator's CLI complexities for different architectures.

# Lesson 3: The World Specification (YAML) and `yaml2qemu`

Welcome to Lesson 3! In this tutorial, you will learn how virtmcu utilizes its Single Source of Truth—the World Specification YAML—to orchestrate QEMU via Device Trees.

## The Problem
In Lesson 1, we manually wrote a Device Tree Source (`.dts`) file to instantiate our machine. While Device Trees are powerful and standard in the Linux kernel world, they are incredibly verbose and focus heavily on physical bus addressing rather than high-level system architecture or multi-node topologies.

## The Solution: `yaml2qemu`
To provide a clean, declarative syntax that scales to multi-node distributed simulations, VirtMCU mandates the use of a hierarchical **YAML format**. This format is designed to map 1:1 with **OpenUSD (Universal Scene Description)**, the industry standard for Digital Twins.

The `yaml2qemu` tool performs a rigorous, validated pipeline:
1. **Parser and Validator**: It parses the YAML using AST models derived strictly from our TypeSpec schema. 
2. **Emitter**: It translates the AST into a valid QEMU Device Tree (`.dts`), injecting required QEMU-specific scaffolding like `qemu:system-memory`.
3. **Compiler**: It invokes `dtc` to produce the binary `.dtb` blob that `arm-generic-fdt` expects.
4. **Post-Compilation Validation**: It disassembles the `.dtb` to verify that every device defined in YAML has a valid memory mapping, preventing silent "Data Abort" crashes.

## Part 1: Try the Translator

In the `src/` directory, there is a `test_board.yaml` file describing our Cortex-A15 board with a PL011 UART.

Run the translation tool using the Python module invocation:
```bash
python3 -m tools.yaml2qemu tutorial/lesson03-world-specification/src/test_board.yaml --out-dtb test_board.dtb --print-cmd
```

## Part 2: Polymorphic Launching

The `virtmcu-run` script is "polymorphic"—it detects the file type you pass and compiles it if necessary.

### 1. Booting via YAML (The Standard)
Pass the `.yaml` file directly, and `virtmcu-run` will invoke `yaml2qemu` for you:
```bash
./target/release/virtmcu-run --yaml tutorial/lesson03-world-specification/src/test_board.yaml --kernel tests/fixtures/guest_apps/boot_arm/hello.elf -nographic
```

### 2. Booting via Native Device Tree (DTS)
If you prefer raw standard Linux Device Tree source, `virtmcu-run` will call the `dtc` compiler automatically:
```bash
./target/release/virtmcu-run --dts tests/fixtures/guest_apps/boot_arm/minimal.dts --kernel tests/fixtures/guest_apps/boot_arm/hello.elf -nographic
```

### 3. Booting via Binary Blob (DTB)
Finally, if you have a pre-compiled blob, it can be loaded directly with no translation overhead:
```bash
./target/release/virtmcu-run --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb --kernel tests/fixtures/guest_apps/boot_arm/hello.elf -nographic
```

## Summary
You have successfully learned how virtmcu uses the World Specification YAML to drive dynamic emulation, providing a future-proof, easily validated, and OpenUSD-aligned platform for digital twins.
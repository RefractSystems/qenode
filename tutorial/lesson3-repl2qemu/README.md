# Lesson 3: Parsing Renode `.repl` Platforms to QEMU Device Trees

Welcome to Lesson 3! In this tutorial, you will learn how qenode bridges the gap between Renode's human-readable platform descriptions (`.repl` files) and QEMU's internal Object Model.

## The Problem
In Lesson 1, we manually wrote a Device Tree Source (`.dts`) file to instantiate our machine. While Device Trees are powerful and standard in the Linux kernel world, they are incredibly verbose and focus heavily on physical bus addressing rather than high-level system architecture.

Renode uses a much cleaner, indentation-based format called REPL (REnode PLatform).
```repl
memory: Memory.MappedMemory @ sysbus 0x40000000
    size: 0x08000000

uart0: UART.PL011 @ sysbus 0x09000000
    -> gic@1
```

## The Solution: `repl2qemu`
To get the best of both worlds—Renode's clean syntax and QEMU's execution speed—we developed the `repl2qemu` offline translation tool.

It performs a three-step pipeline:
1. **Parser (`parser.py`)**: Uses a regex-based state machine to extract devices, memory addresses, properties, and interrupt mappings from the `.repl` file, ignoring complex inline initializations.
2. **Emitter (`fdt_emitter.py`)**: Translates the parsed AST into a valid QEMU Device Tree (`.dts`), mapping Renode class names (e.g., `UART.PL011`) directly to QEMU QOM type names (e.g., `pl011`). It injects required QEMU-specific scaffolding like `qemu:system-memory`.
3. **Compiler**: Invokes the standard `dtc` (Device Tree Compiler) to produce the binary `.dtb` blob that `arm-generic-fdt` expects.

## Part 1: Try the Translator

In the `src/` directory, there is a `test_board.repl` file that describes our standard Cortex-A15 board with 128MB of RAM and a PL011 UART.

Run the translation tool using the Python module invocation from the repository root:
```bash
source .venv/bin/activate
python3 -m tools.repl2qemu tutorial/lesson3-repl2qemu/src/test_board.repl --out-dtb test_board.dtb --print-cmd
```

You will see output indicating that the devices were parsed, the DTS was generated, and compiled. It also prints the equivalent QEMU command line!

## Part 2: Polymorphic Launching

The `run.sh` script is "polymorphic"—it automatically detects the file type you pass and performs the necessary steps to get it into QEMU.

### 1. Booting via REPL
Pass the `.repl` file directly, and `run.sh` will call the translator for you:
```bash
./scripts/run.sh --repl tutorial/lesson3-repl2qemu/src/test_board.repl --kernel test/phase1/hello.elf -nographic
```

### 2. Booting via Native Device Tree (DTS)
If you aren't a Renode user, you can pass a standard Linux Device Tree source. `run.sh` will call the `dtc` compiler automatically:
```bash
./scripts/run.sh --dts test/phase1/minimal.dts --kernel test/phase1/hello.elf -nographic
```

### 3. Booting via Binary Blob (DTB)
Finally, if you have a pre-compiled blob, it can be loaded directly with no overhead:
```bash
./scripts/run.sh --dtb test/phase1/minimal.dtb --kernel test/phase1/hello.elf -nographic
```

In all three cases, you should see `HI` printed to the console!

## Part 3: The Future (YAML & OpenUSD)

While `.repl` provides legacy parity with Renode, it is a bespoke format. For the long-term vision of FirmwareStudio, we are adopting a **modern YAML format** designed to map 1:1 with **OpenUSD (Universal Scene Description)**.

OpenUSD is the industry standard for Digital Twins, allowing physics and cyber-nodes to live in the same hierarchical file.

### 1. Migrate a legacy REPL to YAML
You can instantly modernize any Renode file using our migration tool:
```bash
python3 -m tools.repl2yaml tutorial/lesson3-repl2qemu/src/test_board.repl --out test_board.yaml
```

### 2. Booting via YAML
Just like other formats, `run.sh` supports YAML natively:
```bash
./scripts/run.sh --yaml test_board.yaml --kernel test/phase1/hello.elf -nographic
```

## Summary
You have successfully learned how qenode provides a flexible, future-proof frontend. Whether you are migrating from Renode's legacy `.repl`, using industry-standard `.dts`, or adopting our modern OpenUSD-aligned YAML, the underlying QEMU engine provides high-performance dynamic emulation for your digital twin.
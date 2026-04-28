# Lesson 15: Setup, Distribution, and Pre-built Binaries

Welcome to Lesson 15! Until now, we have been compiling QEMU and the virtmcu peripherals from source. In a professional production or research environment, you often want to distribute the simulation environment to colleagues or CI/CD pipelines without requiring a 40-minute compilation step on every machine.

In this lesson, you will learn how to:
1.  Install the **virtmcu-tools** Python package from PyPI.
2.  Use pre-built **virtmcu QEMU binaries**.
3.  Set up a portable simulation environment.

## Terminology Check

*   **Binary Distribution**: Providing pre-compiled executable files and libraries so users don't need a compiler or the source code.
*   **Wheel (.whl)**: The standard format for distributing Python packages.
*   **RPATH**: A field in a Linux executable that tells it where to find its shared libraries (`.so` files) relative to its own location.
*   **Standalone**: A tool that can run independently of the main development repository.

## Part 1: Installing virtmcu-tools

The `repl2qemu`, `yaml2qemu`, and `virtmcu-mcp` tools are now packaged as a standard Python library. This means you can integrate them into your own Python scripts or use them as CLI commands anywhere on your system.

### Installation

```bash
# It is always recommended to use a virtual environment
python3 -m venv .venv
source .venv/bin/activate

# Install the tools (assuming they are published to PyPI or using the local wheel)
pip install virtmcu-tools
```

Once installed, you can verify the tools are available:

```bash
yaml2qemu --help
repl2qemu --help
virtmcu-mcp --help
```

## Part 2: Using Pre-built QEMU Binaries

The virtmcu project provides automated binary releases for Linux (x86_64 and AArch64). These releases include:
- `qemu-system-arm` / `qemu-system-riscv64` (The patched emulator).
- `hw-virtmcu-*.so` (The dynamic QOM peripheral plugins).
- `libzenohc.so` (The Zenoh transport library).

### Layout of a Binary Release

When you download and extract a `virtmcu-qemu-*.tar.gz` release, you get a structure like this:

```text
virtmcu-dist/
├── bin/
│   ├── qemu-system-arm
│   └── qemu-system-riscv64
├── lib/
│   ├── libzenohc.so
│   └── qemu/
│       └── hw-transport-zenoh.so  <-- The plugins!
└── README-BINARY.md
```

### 🧠 Under the Hood: Portable RPATHs
Normally, Linux binaries look for libraries in `/usr/lib`. Our binaries are "relocatable." We use a trick called `$ORIGIN` in the RPATH. This tells `qemu-system-arm` to look for its libraries in `../lib/` relative to where the binary sits. This means you can move the `virtmcu-dist` folder anywhere on your disk and it will still work!

## Part 3: Running a Simulation from Binaries

Let's use the `yaml2qemu` tool we just installed to generate a board, and then run it with the pre-compiled QEMU.

1.  **Generate the hardware description**:
    ```bash
    yaml2qemu src/board.yaml --out-dtb board.dtb
    ```

2.  **Run with the pre-built binary**:
    If you have extracted the binary release to `./virtmcu-dist`, you would run:
    ```bash
    ./virtmcu-dist/bin/qemu-system-arm -M arm-generic-fdt -dtb board.dtb -nographic
    ```

## Exercises to Try

1.  **Inspect Dependencies**: Run `ldd ./virtmcu-dist/bin/qemu-system-arm`. Look at the entry for `libzenohc.so`. Notice how it points to the library inside your local folder, not a system path!
2.  **MCP Integration**: Start the MCP server with `virtmcu-mcp`. This tool allows AI agents (like Claude or Gemini) to "see" your simulation and interact with it using the Model Context Protocol.
3.  **CI Pipeline Simulation**: Try creating a small script that downloads the binary tarball, installs the tools, and boots a board. This is exactly how we recommend setting up GitHub Actions or GitLab CI for firmware testing.

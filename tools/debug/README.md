# VirtMCU Debugging Tools

This directory contains scripts and helpers for interactive debugging of VirtMCU using GDB.

## Python GDB Helpers

GDB has deep Python integration, allowing you to script complex breakpoints, inspect memory layout, and traverse QOM structures automatically.

### Usage

1.  Launch QEMU under GDB (you can use `QEMU_BIN` to find the correct binary):
    ```bash
    # e.g., using the resolved binary path directly:
    gdb --args ./third_party/qemu/build-virtmcu/qemu-system-arm -M arm-generic-fdt ...
    ```

2.  Inside the GDB prompt, source the Python helper script:
    ```gdb
    (gdb) source tools/debug/gdb_helpers.py
    ```

3.  You can now invoke the helper functions directly from the Python interactive prompt within GDB:
    ```gdb
    (gdb) python setup_tracing()
    (gdb) python trace_function("pl022_realize")
    (gdb) run
    ```

This approach allows you to perform deep inspection without hardcoding your debug scripts to specific bugs or manually stepping through hundreds of iterations.

#!/usr/bin/env python3
"""
apply_zenoh_hook.py — Inject a TCG quantum hook into QEMU's cpu-exec.c.

This allows external QOM modules (like the zenoh clock sync) to halt QEMU
at translation block boundaries.
"""

import os
import sys

def patch_file(path, marker, insertion, after=True):
    with open(path) as f:
        content = f.read()
    if marker not in content:
        print(f"  WARNING: marker not found in {os.path.relpath(path)}: {marker!r}")
        return False
    if insertion.strip() in content:
        return False
    if after:
        content = content.replace(marker, marker + insertion, 1)
    else:
        content = content.replace(marker, insertion + marker, 1)
    with open(path, "w") as f:
        f.write(content)
    print(f"  patched {os.path.relpath(path)}")
    return True

def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = os.path.abspath(sys.argv[1])
    
    cpu_exec_c = os.path.join(qemu, "accel", "tcg", "cpu-exec.c")
    
    # 1. Add the function pointer definition
    marker1 = "/* main execution loop */"
    insertion1 = "\nvoid (*qenode_tcg_quantum_hook)(CPUState *cpu) = NULL;\n"
    patch_file(cpu_exec_c, marker1, insertion1, after=True)
    
    # 2. Add the hook invocation in cpu_exec_loop
    marker2 = "while (!cpu_handle_interrupt(cpu, &last_tb)) {"
    insertion2 = "\n        if (qenode_tcg_quantum_hook) {\n            qenode_tcg_quantum_hook(cpu);\n        }\n"
    patch_file(cpu_exec_c, marker2, insertion2, after=True)

if __name__ == "__main__":
    main()

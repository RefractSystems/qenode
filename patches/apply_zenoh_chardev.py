#!/usr/bin/env python3
import os
import sys


def patch_file(filepath, marker, insertion, after=False):
    with open(filepath, "r") as f:
        content = f.read()
    if insertion in content:
        return
    idx = content.find(marker)
    if idx == -1:
        print(f"Error: Could not find marker '{marker}' in {filepath}")
        sys.exit(1)
    if after:
        idx += len(marker)
    new_content = content[:idx] + insertion + content[idx:]
    with open(filepath, "w") as f:
        f.write(new_content)


def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = os.path.abspath(sys.argv[1])
    char_c = os.path.join(qemu, "chardev", "char.c")

    # 1. Patch qemu_chardev_opts in chardev/char.c
    marker4 = """        },{
            .name = "size","""
    insertion4 = """        },{
            .name = "node",
            .type = QEMU_OPT_STRING,
        },{
            .name = "router",
            .type = QEMU_OPT_STRING,
        },{
            .name = "topic",
            .type = QEMU_OPT_STRING,"""
    patch_file(char_c, marker4, insertion4, after=False)


if __name__ == "__main__":
    main()

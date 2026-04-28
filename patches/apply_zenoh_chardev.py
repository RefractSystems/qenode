#!/usr/bin/env python3
import re
import sys
from pathlib import Path


def patch_file(filepath, marker_pattern, insertion, after=False):
    with Path(filepath).open() as f:
        content = f.read()
    if insertion in content:
        return False

    match = re.search(marker_pattern, content)
    if not match:
        print(f"Error: Could not find marker pattern '{marker_pattern}' in {filepath}")
        sys.exit(1)

    idx = match.start()
    if after:
        idx = match.end()

    new_content = content[:idx] + insertion + content[idx:]
    with Path(filepath).open("w") as f:
        f.write(new_content)
    return True


def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = Path(sys.argv[1]).resolve()
    char_c = Path(qemu) / "chardev" / "char.c"

    # 1. Patch qemu_chardev_opts in chardev/char.c
    # Use regex to match .name = "size" with any whitespace/tabs
    marker_pattern = r'\.name\s*=\s*"size",'
    insertion4 = """.name = "node",
            .type = QEMU_OPT_STRING,
        },{
            .name = "transport",
            .type = QEMU_OPT_STRING,
        },{
            .name = "router",
            .type = QEMU_OPT_STRING,
        },{
            .name = "topic",
            .type = QEMU_OPT_STRING,
        },{
            .name = "max-backlog",
            .type = QEMU_OPT_SIZE,
        },{
            """
    # Use a more specific check for the whole block to avoid double patching
    content = Path(char_c).read_text()
    if ".name = \"max-backlog\"," not in content and patch_file(char_c, marker_pattern, insertion4, after=False):
        print(f"  patched {char_c}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
import logging
import re
import sys
from pathlib import Path

logger = logging.getLogger(__name__)


def patch_file(filepath: str | Path, marker_pattern: str, insertion: str, after: bool = False) -> bool:
    with Path(filepath).open() as f:
        content = f.read()
    if insertion in content:
        return False

    match = re.search(marker_pattern, content)
    if not match:
        logger.error(f"Error: Could not find marker pattern '{marker_pattern}' in {filepath}")
        sys.exit(1)

    idx = match.start()
    if after:
        idx = match.end()

    new_content = content[:idx] + insertion + content[idx:]
    with Path(filepath).open("w") as f:
        f.write(new_content)
    return True


def main() -> None:
    if len(sys.argv) != 2:
        logger.info(f"Usage: {sys.argv[0]} <qemu-source-dir>")
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
            .name = "baud-rate-ns",
            .type = QEMU_OPT_NUMBER,
        },{
            """
    # Use a more specific check for the whole block to avoid double patching
    content = Path(char_c).read_text()
    if '.name = "max-backlog",' not in content and patch_file(char_c, marker_pattern, insertion4, after=False):
        logger.info(f"  patched {char_c}")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()

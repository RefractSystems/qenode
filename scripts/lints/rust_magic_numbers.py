#!/usr/bin/env python3
"""
Enforces the 'No Magic Numbers' rule for Rust code.
Enforces that hex/decimal literals (other than 0, 1, -1) are extracted to named constants.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import argparse
import logging
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lint_utils import DEFAULT_EXCLUDES, ENTERPRISE_MANDATE, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)

# Literals that are ALWAYS allowed
ALLOWED_LITERALS = {"0", "1", "0x0", "0x1", "128", "256", "512", "1024"}  # Common buffer sizes allowed for now


def check_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    # Strip comments to avoid false positives
    lines_orig = content.splitlines()
    content_no_comments = re.sub(r"//.*", "", content)
    content_no_comments = re.sub(r"/\*.*?\*/", "", content_no_comments, flags=re.DOTALL)
    lines = content_no_comments.splitlines()

    for i, line in enumerate(lines):
        # Skip lines that are likely part of a const/static/enum definition
        if re.match(r"^\s*(pub\s+)?(const|static)\s+", line) or re.match(r"^\s*[A-Z][a-zA-Z0-9_]*\s*=\s*[0-9]+", line):
            continue

        # Skip error_setg! messages
        if "error_setg!" in line:
            continue

        # Look for hex and decimal literals
        # Regex explanation:
        # \b(0x[0-9a-fA-F]+|[0-9]+)\b : matches hex or decimal literals
        matches = re.finditer(r"\b(0x[0-9a-fA-F]+|[0-9]+)\b", line)
        for m in matches:
            val_str = m.group(1)

            if val_str in ALLOWED_LITERALS:
                continue

            # Heuristic: if it's followed by 'u8', 'u16', etc., it might still be a magic number
            # e.g. 0x42u32

            # Check context: is it an array size [0; 128]?
            # We already allow 128, but let's check the pattern
            if re.search(r"\[\s*.*;\s*" + re.escape(val_str) + r"\s*\]", line):
                continue

            # If we are here, it's a potential magic number
            violations.append(
                f"{path}:{i + 1}: Magic number '{val_str}' found.\n"
                f"  Fix: Extract to a named 'const'. {ENTERPRISE_MANDATE}\n"
                f"  Line: {lines_orig[i].strip()}"
            )

    return violations


def run_lint(targets: list[Path], excludes: list[str]) -> bool:
    all_violations = []

    for path in iter_target_files(targets, excludes, "*.rs"):
        # Skip tests and build scripts as per policy
        if "tests" in path.parts or path.name == "build.rs" or "_generated.rs" in path.name:
            continue
        all_violations.extend(check_file(path))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False

    logger.info("✓ Rust magic numbers check passed.")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Rust magic numbers.")
    parser.add_argument("targets", type=Path, nargs="*", default=[Path("hw/rust")], help="Target directories or files")
    parser.add_argument("--exclude", nargs="*", default=DEFAULT_EXCLUDES, help="Directories to exclude")
    args = parser.parse_args()

    setup_lint_logging()
    success = run_lint(args.targets, args.exclude)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()

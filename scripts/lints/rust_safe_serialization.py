#!/usr/bin/env python3
"""
Enforces Safe Endianness Serialization for Enterprise-Grade Rust code.
Bans the use of ptr::copy_nonoverlapping for serialization in favor of
explicit and safe byte-order methods (.to_le_bytes() / .from_le_bytes()).
Maintains SOTA software quality and prevents endianness bugs.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import argparse
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))  # noqa: TID251
from lint_utils import DEFAULT_EXCLUDES, ENTERPRISE_MANDATE, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)


def lint_rust_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    lines = content.splitlines()
    for i, line in enumerate(lines):
        if "ptr::copy_nonoverlapping" in line:
            has_exception = "COPY_EXCEPTION:" in line
            if not has_exception and i > 0:
                has_exception = "COPY_EXCEPTION:" in lines[i - 1]
            if not has_exception and i < len(lines) - 1:
                has_exception = "COPY_EXCEPTION:" in lines[i + 1]

            if not has_exception:
                violations.append(
                    f"{path}:{i + 1}: Banned ptr::copy_nonoverlapping found (Safe Endianness Serialization Mandate).\n"
                    f"  Fix: Use .to_le_bytes() / .from_le_bytes() instead. {ENTERPRISE_MANDATE} '// COPY_EXCEPTION: <reason>'."
                )

    return violations


def run_lint(targets: list[Path], excludes: list[str]) -> bool:
    """Executes the linting process. Returns True if passed, False if violations found."""
    all_violations = []

    for path in iter_target_files(targets, excludes, "*.rs"):
        if path.name == "core_generated.rs":
            continue
        all_violations.extend(lint_rust_file(path))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False

    logger.info("✓ Safe Endianness Serialization Rust lint passed.")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Rust Safe Endianness Serialization (ptr::copy_nonoverlapping).")
    parser.add_argument("targets", type=Path, nargs="*", default=[Path("hw/rust")], help="Target directories or files")
    parser.add_argument("--exclude", nargs="*", default=DEFAULT_EXCLUDES, help="Directories to exclude")
    args = parser.parse_args()

    setup_lint_logging()
    success = run_lint(args.targets, args.exclude)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()

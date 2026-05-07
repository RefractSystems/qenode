#!/usr/bin/env python3
"""
Enforces the ban on static state in Rust peripherals (Mandate #5).
Static state in DSO plugins is dangerous because it leads to state duplication
if the plugin is loaded multiple times or compiled into separate shared libraries.

All peripheral state MUST live in the QEMU Object (struct) and be managed via RAII.
Centralized state must be exported from the main QEMU binary via VIRTMCU_EXPORT.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import argparse
import logging
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lint_utils import (
    DEFAULT_EXCLUDES,
    ENTERPRISE_MANDATE,
    is_suppressed,
    iter_target_files,
    setup_lint_logging,
)

logger = logging.getLogger(__name__)

RULE_NAME = "static_state"

# Banned state types
BANNED_TYPES = ["Atomic", "Mutex", "OnceCell", "OnceLock", "RwLock", "Cell<", "RefCell<"]


def check_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    lines = content.splitlines()

    # 1. Banned macros (lazy_static!, thread_local!)
    for macro in ["lazy_static!", "thread_local!"]:
        for match in re.finditer(re.escape(macro), content):
            line_no = content.count("\n", 0, match.start()) + 1
            line = lines[line_no - 1]
            # Check current line and previous line for suppression
            suppressed = is_suppressed(line, RULE_NAME) or (
                line_no > 1 and is_suppressed(lines[line_no - 2], RULE_NAME)
            )

            if suppressed:
                continue
            violations.append(
                f"{path}:{line_no}: Banned macro detected ('{macro}').\n"
                f"  Fix: Move state into the peripheral struct. {ENTERPRISE_MANDATE} '// virtmcu-allow: {RULE_NAME} reasoning=\"<reason>\"'."
            )

    # 2. Multi-line regex for static declarations
    # matches `static NAME: TYPE [= ...];` or `static mut NAME: TYPE [= ...];`
    static_regex = re.compile(r"static\s+(mut\s+)?[A-Z0-9_]+\s*:\s*([^=;]+)", re.MULTILINE)
    for match in static_regex.finditer(content):
        type_sig = match.group(2)
        # If it's `static mut`, it's an immediate violation unless it's a known safe C-FFI exception
        is_mut = match.group(1) is not None

        has_banned_type = any(t in type_sig for t in BANNED_TYPES)

        if is_mut or has_banned_type:
            line_no = content.count("\n", 0, match.start()) + 1
            line = lines[line_no - 1]

            # Determine the full declaration block to check for inline suppression
            start_idx = match.start()
            semi_idx = content.find(";", start_idx)
            if semi_idx == -1:
                semi_idx = len(content)

            decl_block = content[start_idx:semi_idx]

            # Check current line, previous line, or the declaration block itself
            suppressed = (
                is_suppressed(line, RULE_NAME)
                or (line_no > 1 and is_suppressed(lines[line_no - 2], RULE_NAME))
                or is_suppressed(decl_block, RULE_NAME)
            )

            if suppressed:
                continue

            violations.append(
                f"{path}:{line_no}: Banned static state detected in type: {type_sig.strip()}.\n"
                f"  Fix: Move state into the peripheral struct or export from main binary. {ENTERPRISE_MANDATE} '// virtmcu-allow: {RULE_NAME} reasoning=\"<reason>\"'."
            )

    return violations


def run_lint(targets: list[Path], excludes: list[str]) -> bool:
    """Executes the linting process. Returns True if passed, False if violations found."""
    all_violations = []

    for path in iter_target_files(targets, excludes, "*.rs"):
        all_violations.extend(check_file(path))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False

    logger.info("✓ Rust static state ban lint passed.")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Rust static state in peripherals.")
    parser.add_argument("targets", type=Path, nargs="*", default=[Path("hw/rust")], help="Target directories")
    parser.add_argument("--exclude", nargs="*", default=DEFAULT_EXCLUDES, help="Directories to exclude")
    args = parser.parse_args()

    setup_lint_logging()
    success = run_lint(args.targets, args.exclude)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()

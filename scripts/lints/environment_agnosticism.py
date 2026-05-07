#!/usr/bin/env python3
"""
Enforces environment agnosticism by banning absolute paths and user-specific home directories.
This script scans tools and tests for hardcoded paths like /home/, /Users/, /tmp/, and ~/.

Mandate: All paths MUST be relative or constructed via platform-agnostic APIs (e.g., os.path.join).
"""

import argparse
import logging
import re
import sys
from pathlib import Path

# Setup path so we can import our lint_utils regardless of execution context
sys.path.insert(0, str(Path(__file__).resolve().parent))
from lint_utils import DEFAULT_EXCLUDES, is_suppressed, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)

# Patterns to flag: (Pattern, Message, Fix Suggestion)
# We look for paths in strings or as standalone absolute paths.
ENVIRONMENT_LINTS = [
    (
        r"['\"]/home/|['\"]/Users/|['\"]~/|/home/vscode",
        "User-specific or absolute home path detected.",
        "Use relative paths or environment variables (e.g., $HOME).",
    ),
    (
        r"['\"]/tmp/|/tmp/[a-zA-Z0-9_-]+",
        "Hardcoded /tmp/ path detected.",
        "Use tempfile module (Python), tempfile crate (Rust), or $TMPDIR (Shell).",
    ),
    (
        r"(?i)(?<![a-z])[a-z]:\\[^ \n\r\t]",
        "Windows-style absolute path detected.",
        "Use platform-agnostic path construction.",
    ),
    (
        r"\.venv/|bin/activate",
        "Virtual environment reference detected.",
        "The project mandate forbids venvs. Use system-wide Python or uv run --no-project.",
    ),
]

RULE_NAME = "absolute_path"

# Files to scan
TARGET_EXTENSIONS = ["*.py", "*.rs", "*.sh", "*.c", "*.h", "*.cpp", "*.yaml", "*.yml", "*.dts"]
TARGET_DIRS = ["tools", "tests"]


def strip_comments(line: str, ext: str) -> str:
    if ext in [".py", ".sh", ".yaml", ".yml"]:
        return line.split("#", maxsplit=1)[0]
    elif ext in [".rs", ".c", ".cpp", ".h", ".dts"]:
        return line.split("//", maxsplit=1)[0].split("/*", maxsplit=1)[0]
    return line


def check_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    if is_suppressed(content[:1000], RULE_NAME):
        return []

    lines = content.splitlines()
    ext = path.suffix

    for pattern, msg, fix in ENVIRONMENT_LINTS:
        regex = re.compile(pattern)
        for i, line in enumerate(lines):
            # Skip lines that use the safe cross-platform TMPDIR fallback
            if "${TMPDIR:-/tmp}" in line:
                continue

            clean_line = strip_comments(line, ext)
            if regex.search(clean_line):
                # Check for line-level exception
                if is_suppressed(line, RULE_NAME):
                    continue
                violations.append(f"{path}:{i + 1}: {msg}\n  Fix: {fix}\n  Line: {line.strip()}")

    return violations


def run_lint(targets: list[Path], excludes: list[str]) -> bool:
    """Executes the linting process. Returns True if passed, False if violations found."""
    all_violations = []

    for ext in TARGET_EXTENSIONS:
        for path in iter_target_files(targets, excludes, ext):
            # Skip this script itself
            if path.name == "environment_agnosticism.py":
                continue
            all_violations.extend(check_file(path))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False

    logger.info("✓ Environment agnosticism check passed.")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Lint for environment agnosticism (absolute paths).")
    parser.add_argument(
        "targets", type=Path, nargs="*", default=[Path(d) for d in TARGET_DIRS], help="Target directories or files"
    )
    parser.add_argument("--exclude", nargs="*", default=DEFAULT_EXCLUDES, help="Directories to exclude")
    args = parser.parse_args()

    setup_lint_logging()
    success = run_lint(args.targets, args.exclude)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()

"""
Common utilities and constants for VirtMCU linting scripts.

Designed for reuse: Provides location-agnostic path resolution for parent repositories.
"""

import logging
import re
from collections.abc import Iterable
from pathlib import Path

logger = logging.getLogger(__name__)

ENTERPRISE_MANDATE = (
    "ENTERPRISE MANDATE: Fix the underlying issue. Masking it with a comment escape "
    "is an absolute last resort, strictly violates SOTA software quality, and will be "
    "forced to be removed during review. Only if structurally impossible to fix, use"
)

DEFAULT_EXCLUDES = [
    "third_party",
    "build",
    "target",
    ".git",
    ".github",
    ".claude",
    "__pycache__",
    ".cargo-cache",
]


def setup_lint_logging() -> None:
    """Set up standard logging for lint scripts."""
    logging.basicConfig(level=logging.INFO, format="%(message)s")


def is_suppressed(line: str, rule_name: str) -> bool:
    """
    Checks if a lint rule is suppressed on a given line.
    Enforces the SOTA 'virtmcu-allow: <rule_name> reasoning="<reason>"' pattern.
    """
    pattern = rf'virtmcu-allow:\s*{rule_name}\s+reasoning="[^"]+"'
    return bool(re.search(pattern, line))


def iter_target_files(targets: list[Path], excludes: list[str], pattern: str) -> Iterable[Path]:
    """
    Yields all files matching `pattern` in `targets` that do not contain
    any of the `excludes` in their path parts relative to the target.
    """
    for target in targets:
        if not target.exists():
            logger.warning(f"Target path does not exist: {target}")
            continue

        # If target is a file, yield it directly if it matches the extension
        if target.is_file():
            if target.match(pattern):
                yield target
            continue

        # If target is a directory, walk it
        for path in target.rglob(pattern):
            # Check exclusions relative to the target to allow linting submodules
            # that might be located in an 'excluded' directory (e.g. third_party/virtmcu)
            try:
                relative_path = path.relative_to(target)
                if any(p in relative_path.parts for p in excludes):
                    continue
            except ValueError:
                # If path is not relative to target (should not happen with rglob), skip it
                continue
            yield path

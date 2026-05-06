"""
Common utilities and constants for VirtMCU linting scripts.
"""
import logging
from collections.abc import Iterable
from pathlib import Path

logger = logging.getLogger(__name__)

ENTERPRISE_MANDATE = (
    "ENTERPRISE MANDATE: Fix the underlying issue. Masking it with a comment escape "
    "is an absolute last resort, strictly violates SOTA software quality, and will be "
    "forced to be removed during review. Only if structurally impossible to fix, use"
)

DEFAULT_EXCLUDES = [
    ".venv", 
    ".venv-docker", 
    "third_party", 
    "build", 
    "target", 
    ".git", 
    ".claude", 
    "__pycache__", 
    ".cargo-cache"
]

def setup_lint_logging() -> None:
    """Set up standard logging for lint scripts."""
    logging.basicConfig(level=logging.INFO, format="%(message)s")

def iter_target_files(targets: list[Path], excludes: list[str], pattern: str) -> Iterable[Path]:
    """
    Yields all files matching `pattern` in `targets` that do not contain 
    any of the `excludes` in their path parts.
    """
    for target in targets:
        if not target.exists():
            logger.warning(f"Target path does not exist: {target}")
            continue

        # If target is a file, yield it directly if it matches the extension
        if target.is_file():
            if target.match(pattern) and not any(p in target.parts for p in excludes):
                yield target
            continue

        # If target is a directory, walk it
        for path in target.rglob(pattern):
            if any(p in path.parts for p in excludes):
                continue
            yield path

#!/usr/bin/env python3
"""
SOTA Lint: Dependency Pinning
Enforces that all Python dependencies in pyproject.toml are pinned to a specific version (==).

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import logging
import sys
import tomllib
from pathlib import Path

# Add the workspace root to sys.path to allow absolute imports from scripts
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent))

from scripts.lints.lint_utils import ENTERPRISE_MANDATE, setup_lint_logging

logger = logging.getLogger(__name__)


def check_dependencies(deps: list[str], group_name: str) -> list[str]:
    """Check if dependencies are pinned to an exact version."""
    errors = []
    for dep in deps:
        # Basic check: must contain '=='
        if "==" not in dep:
            errors.append(f"Dependency '{dep}' in group '{group_name}' is not pinned with '=='")

        # Additional check: should not have other specifiers like >=, <=, ~, ^
        elif any(op in dep.split("==")[0] for op in [">", "<", "~", "^"]):
            errors.append(f"Dependency '{dep}' in group '{group_name}' contains illegal specifiers before '=='")

    return errors


def main() -> None:
    """Main entry point for the dependency pinning lint."""
    setup_lint_logging()
    pyproject_path = Path("pyproject.toml")

    if not pyproject_path.exists():
        logger.error("pyproject.toml not found")
        sys.exit(1)

    try:
        with pyproject_path.open("rb") as f:
            data = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        logger.error(f"Failed to parse pyproject.toml: {e}")
        sys.exit(1)

    all_errors = []

    # Check project.dependencies
    project_deps = data.get("project", {}).get("dependencies", [])
    all_errors.extend(check_dependencies(project_deps, "project.dependencies"))

    # Check dependency-groups
    dep_groups = data.get("dependency-groups", {})
    for group_name, deps in dep_groups.items():
        all_errors.extend(check_dependencies(deps, f"dependency-groups.{group_name}"))

    if all_errors:
        logger.error("════════════════════════════════════════════════════")
        logger.error("  LINT FAILURE: Unpinned Dependencies Detected")
        logger.error("════════════════════════════════════════════════════")
        logger.error(ENTERPRISE_MANDATE)
        logger.error("All dependencies MUST be pinned to an exact version using '=='.")
        logger.error("")
        for err in all_errors:
            logger.error(f"  - {err}")
        logger.error("")
        sys.exit(1)

    logger.info("✓ All dependencies are pinned.")


if __name__ == "__main__":
    main()

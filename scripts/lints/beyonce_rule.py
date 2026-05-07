#!/usr/bin/env python3
"""
Beyoncé Rule Verification ("If you liked it, you should have put a test on it")

Mandate: Any code modifications in 'hw/rust/' or 'tools/' must be accompanied
by corresponding changes in the 'tests/' directory.
"""

import logging
import os
import shutil
import subprocess
import sys
from pathlib import Path

# Files or extensions that are exempt from the Beyoncé Rule
EXEMPT_EXTENSIONS = {".md", ".txt", ".png", ".jpg", ".svg", ".json"}
EXEMPT_FILES = {"Cargo.toml", "Cargo.lock", "uv.lock", "pyproject.toml", "BUILD_DEPS", "VERSION", "build.rs"}

logger = logging.getLogger(__name__)


def get_git_executable() -> str:
    """Find the full path to the git executable."""
    git_path = shutil.which("git")
    if not git_path:
        logger.error("❌ ERROR: 'git' executable not found in PATH.")
        sys.exit(1)
    return git_path


def setup_logging() -> None:
    """Set up standard logging for lint scripts."""
    logging.basicConfig(level=logging.INFO, format="%(message)s")


def is_exempt(file_path: str) -> bool:
    """Check if a file is exempt from the Beyoncé Rule."""
    path = Path(file_path)
    if path.suffix in EXEMPT_EXTENSIONS:
        return True
    return path.name in EXEMPT_FILES


def get_changed_files() -> list[str]:
    """Get the list of changed files compared to the base branch, including dirty working tree changes."""
    changed_files = set()

    # In CI (GitHub Actions), GITHUB_BASE_REF is the target branch of the PR.
    base_ref = os.environ.get("GITHUB_BASE_REF")

    git = get_git_executable()

    if not base_ref:
        # Locally, we try to compare against origin/main or main
        for candidate in ["origin/main", "main"]:
            try:
                subprocess.run(
                    [git, "rev-parse", "--verify", candidate],
                    capture_output=True,
                    check=True,
                )
                base_ref = candidate
                break
            except subprocess.CalledProcessError:
                continue

    if not base_ref:
        logger.error("❌ ERROR: Could not determine base branch (tried GITHUB_BASE_REF, origin/main, main).")
        logger.error("   Cannot verify Beyoncé Rule. This is a fatal error in CI.")
        logger.error("   Ensure your repository is fetched (e.g., fetch-depth: 0 in GitHub Actions).")
        sys.exit(1)

    try:
        # 1. Get changes committed to the current branch since it diverged from base
        # We use --diff-filter=d to ignore files that were purely deleted (deleting code doesn't require new tests).
        result = subprocess.run(
            [git, "diff", f"{base_ref}...HEAD", "--name-only", "--diff-filter=d"],
            capture_output=True,
            text=True,
            check=True,
        )
        for line in result.stdout.splitlines():
            if line.strip():
                changed_files.add(line.strip())

        # 2. Get staged changes (not yet committed)
        staged_result = subprocess.run(
            [git, "diff", "--staged", "--name-only", "--diff-filter=d"],
            capture_output=True,
            text=True,
            check=True,
        )
        for line in staged_result.stdout.splitlines():
            if line.strip():
                changed_files.add(line.strip())

        # 3. Get unstaged changes (dirty working tree)
        unstaged_result = subprocess.run(
            [git, "diff", "--name-only", "--diff-filter=d"],
            capture_output=True,
            text=True,
            check=True,
        )
        for line in unstaged_result.stdout.splitlines():
            if line.strip():
                changed_files.add(line.strip())

    except subprocess.CalledProcessError as e:
        logger.error("❌ ERROR: Git diff failed: %s", e)
        sys.exit(1)

    return list(changed_files)


def main() -> None:
    """Main execution point for the Beyoncé Rule check."""
    import argparse
    parser = argparse.ArgumentParser(description="Verify the Beyoncé Rule (code changes require test changes).")
    parser.add_argument("--watch", nargs="+", default=["hw/rust/", "tools/"], help="Directories to watch for code changes")
    parser.add_argument("--test-dir", default="tests/", help="Directory where tests should be added/updated")
    args = parser.parse_args()

    setup_logging()
    logger.info("==> Verifying Beyoncé Rule...")

    changed_files = get_changed_files()
    if not changed_files:
        logger.info("✓ No changes detected or base branch unavailable.")
        return

    # Filter for relevant changes in watch directories
    code_changes = [
        f for f in changed_files if any(f.startswith(w) for w in args.watch) 
        and not f.startswith("tools/testing/")
        and not f.startswith("tools/debug/")
        and not is_exempt(f)
    ]

    # Check if any tests were changed
    tests_changed = any(f.startswith(args.test_dir) for f in changed_files)

    if code_changes and not tests_changed:
        logger.error("❌ BEYONCÉ RULE VIOLATION: Code changes detected without corresponding test changes.")
        logger.error("   Mandate: 'If you liked it, you should have put a test on it.'")
        logger.error(f"\n   The following files were modified, but no changes were found in '{args.test_dir}':")
        for f in code_changes:
            logger.error("     - %s", f)
        logger.error("\n   Please add or update integration/unit tests to verify your changes.")
        sys.exit(1)

    if code_changes:
        logger.info(
            "✓ Beyoncé Rule passed: %d code files changed, and tests were updated.",
            len(code_changes),
        )
    else:
        logger.info("✓ Beyoncé Rule passed: No relevant code changes detected.")


if __name__ == "__main__":
    main()

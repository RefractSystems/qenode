#!/usr/bin/env python3
"""
Enforces the ban on direct modifications to third-party submodules.
All changes to third-party code must be maintained as patches in the 'patches/' directory.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import os
import subprocess
import sys
from pathlib import Path


def run_git(args: list[str], cwd: str | None = None) -> str:
    """Run a git command and return its stdout."""
    git_exec = "git"
    try:
        result = subprocess.run(
            [git_exec, *args],
            cwd=cwd,
            capture_output=True,
            text=True,
            check=False,
        )
        return result.stdout.strip()
    except Exception:
        return ""


def check_submodule(sub_path: str) -> bool:
    """Check if a submodule has uncommitted modifications to tracked files."""
    Path(sub_path).name
    Path.cwd()

    if not (Path(sub_path) / ".git").exists():
        # Not a git repo, skip
        return True

    # Check for modifications to tracked files only (-uno ignores untracked build dirs)
    status = run_git(["status", "--porcelain", "-uno"], cwd=sub_path)
    if not status:
        return True  # Clean

    return False


def main() -> None:
    """Main function."""
    if os.environ.get("VIRTMCU_DEVELOPING_PATCH") == "1":
        return

    third_party_dir = Path("third_party")
    if not third_party_dir.exists():
        return

    all_ok = True
    for item in third_party_dir.iterdir():
        if item.is_dir() and not check_submodule(str(item)):
            all_ok = False

    if not all_ok:
        sys.exit(1)


if __name__ == "__main__":
    main()

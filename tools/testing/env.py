"""
SOTA Environment and Path Management for VirtMCU Tests.
Provides centralized path resolution and build automation to eliminate boilerplate.
"""

import os
import subprocess
from pathlib import Path


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent  # Fallback


WORKSPACE_ROOT = _find_workspace_root(Path(__file__).resolve())
TESTS_DIR = WORKSPACE_ROOT / "tests"
FIXTURES_DIR = TESTS_DIR / "fixtures"
GUEST_APPS_DIR = FIXTURES_DIR / "guest_apps"
TOOLS_DIR = WORKSPACE_ROOT / "tools"
SCRIPTS_DIR = WORKSPACE_ROOT / "scripts"
RUN_SH = Path(os.environ.get("RUN_SH") or (SCRIPTS_DIR / "run.sh"))


def build_guest_app(app_name: str) -> Path:
    """
    SOTA Helper: Resolves the absolute path to a guest app and compiles it.
    Returns the path to the application directory so tests don't need to manually
    invoke Makefiles or manage paths.
    """
    app_dir = GUEST_APPS_DIR / app_name
    if not app_dir.exists():
        raise FileNotFoundError(f"Guest app not found: {app_dir}")

    # Check if a Makefile exists before running
    if (app_dir / "Makefile").exists():
        subprocess.run(["make", "-C", str(app_dir), "all"], check=True)
    return app_dir

#!/usr/bin/env python3
"""
Enforces Enterprise-Grade Shell script safety standards.
This script wraps shellcheck and ensures all bash scripts use rigorous safety flags
(set -euo pipefail) to prevent hidden failures and maintain SOTA reliability.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import argparse
import logging
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent)) # noqa: TID251
from lint_utils import DEFAULT_EXCLUDES, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)

def check_shellcheck(targets: list[Path], excludes: list[str]) -> list[str]:
    """Run shellcheck on all bash scripts."""
    try:
        subprocess.run(["shellcheck", "--version"], capture_output=True, check=True) # noqa: S607
    except (subprocess.CalledProcessError, FileNotFoundError):
        logger.error("❌ Error: shellcheck is not installed. Install with: sudo apt-get install shellcheck")
        return [ "shellcheck not installed" ]

    scripts = [str(p) for p in iter_target_files(targets, excludes, "*.sh")]

    if not scripts:
        return []

    violations = []
    # Use -x to follow source statements
    result = subprocess.run(["shellcheck", "--severity=warning"] + scripts, capture_output=True, text=True) # noqa: S603, S607
    if result.returncode != 0:
        violations.append(f"shellcheck violations found:\n{result.stdout}")

    return violations

def check_safety_flags(targets: list[Path], excludes: list[str]) -> list[str]:
    """Check for 'set -euo pipefail' in all bash scripts."""
    violations = []
    
    for path in iter_target_files(targets, excludes, "*.sh"):
        try:
            content = path.read_text()
            if "set -euo pipefail" not in content:
                violations.append(
                    f"{path}: Missing 'set -euo pipefail' safety flags.\n"
                    "  Fix: Add 'set -euo pipefail' at the top of the script (after the shebang)."
                )
        except UnicodeDecodeError:
            continue
            
    return violations

def run_lint(targets: list[Path], excludes: list[str]) -> bool:
    """Executes the linting process. Returns True if passed, False if violations found."""
    all_violations = []
    
    logger.info("==> shellcheck...")
    all_violations.extend(check_shellcheck(targets, excludes))
    
    logger.info("==> Checking bash safety flags (set -euo pipefail)...")
    all_violations.extend(check_safety_flags(targets, excludes))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False
        
    logger.info("✓ Shell lint passed.")
    return True

def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Shell scripts for safety flags and shellcheck violations.")
    parser.add_argument("targets", type=Path, nargs="*", default=[Path(".")], help="Target directories or files")
    parser.add_argument(
        "--exclude", 
        nargs="*", 
        default=DEFAULT_EXCLUDES, 
        help="Directories to exclude"
    )
    args = parser.parse_args()

    setup_lint_logging()
    success = run_lint(args.targets, args.exclude)
    sys.exit(0 if success else 1)

if __name__ == "__main__":
    main()

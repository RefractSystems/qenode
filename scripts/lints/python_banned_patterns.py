#!/usr/bin/env python3
"""
Enforces Enterprise-Grade Python coding standards and checks for banned patterns.
This script scans Python files for forbidden functions, non-deterministic behaviors,
and unsafe abstractions to maintain SOTA software quality. Comment escapes are strictly
forbidden unless structurally unavoidable.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import argparse
import logging
import re
import sys
from pathlib import Path

# Setup path so we can import our lint_utils regardless of execution context
sys.path.insert(0, str(Path(__file__).resolve().parent)) # noqa: TID251
from lint_utils import DEFAULT_EXCLUDES, ENTERPRISE_MANDATE, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)

# (Pattern, Message, Fix Suggestion, [Included Dirs], [Excluded Files/Dirs])
PYTHON_LINTS = [
    (
        r"struct\.(pack|unpack|Struct)|import struct|from struct|Struct\(",
        "Banned struct usage detected.",
        "Use vproto.py or FlatBuffers wrappers for protocol serialization.",
        ["tests", "tools", "docs/tutorials", "scripts"],
        ["proto_gen.py", "vproto.py", "tools/README.md"]
    ),
    (
        r"noqa: TID251",
        "Bypassing TID251 (path bootstrapping ban) is strictly forbidden.",
        "Rely on uv package boundaries instead of manual sys.path manipulation.",
        ["tests", "tools", "scripts"],
        []
    ),
    (
        r"stall-timeout=[0-9]+",
        "Hardcoded stall-timeout detected.",
        "Use dynamic scaling via the VIRTMCU_STALL_TIMEOUT_MS environment variable.",
        ["tests", "tools"],
        []
    ),
    (
        r"(asyncio|time)\.sleep\(",
        "Banned sleep call found.",
        f"Use vta.step() or transport signaling instead. {ENTERPRISE_MANDATE} '# SLEEP_EXCEPTION: <reason>'.",
        ["tests", "tools", "docs/tutorials"],
        ["SLEEP_EXCEPTION"]
    ),
    (
        r"zenoh\.open\(",
        "Raw zenoh.open() found in pytest scope.",
        f"Use make_client_config() / zenoh_session fixture (ADR-014). {ENTERPRISE_MANDATE} '# ZENOH_OPEN_EXCEPTION: <reason>'.",
        ["tests/integration", "tests/unit", "tests/system", "tests", "tools/testing"],
        ["ZENOH_OPEN_EXCEPTION", "fixtures/guest_apps"]
    ),
    (
        r'["\']/(flexray|spi[0-9]|wifi[0-9]|uart[0-9]|memory)["\']',
        "Hardcoded QOM path without unit address detected.",
        "Root FDT devices must use '/device@address' format.",
        ["tests"],
        []
    ),
    (
        r"uuid\.uuid4\(\)",
        "Non-deterministic uuid.uuid4() found in tests.",
        f"Use os.getpid() or the pytest worker_id fixture instead. {ENTERPRISE_MANDATE} '# UUID_EXCEPTION: <reason>'.",
        ["tests"],
        ["UUID_EXCEPTION", "fixtures/guest_apps"]
    ),
    (
        r"\btimeout=[2-9][0-9]{2,}|\btimeout=[0-9]{4,}",
        "Oversized hardcoded timeout (>= 200 s) in tests.",
        f"Use get_time_multiplier() scaling or vta.step(timeout=T). {ENTERPRISE_MANDATE} '# TIMEOUT_EXCEPTION: <reason>'.",
        ["tests"],
        ["TIMEOUT_EXCEPTION", "fixtures/guest_apps"]
    ),
]

def check_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    lines = content.splitlines()

    for pattern, msg, fix, inc_dirs, exc_list in PYTHON_LINTS:
        if not any(inc in str(path) for inc in inc_dirs):
            continue
            
        if any(exc in str(path) for exc in exc_list if "/" in exc or "." in exc):
            continue

        regex = re.compile(pattern)
        for i, line in enumerate(lines):
            if regex.search(line):
                if any(exc in line for exc in exc_list if "/" not in exc and "." not in exc):
                    continue
                if i < 10 and any(exc in line for exc in exc_list):
                    continue
                violations.append(f"{path}:{i+1}: {msg}\n  Fix: {fix}\n  Line: {line.strip()}")

    return violations

def check_direct_zenoh_hacks(path: Path) -> list[str]:
    if "tests/integration/simulation/" not in str(path):
        return []

    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    if "ZENOH_HACK_EXCEPTION" in content[:500]:
        return []

    violations = []
    lines = content.splitlines()
    regex = re.compile(r"^import zenoh|^[ \t]*import zenoh|zenoh_session\b")

    for i, line in enumerate(lines):
        if regex.search(line):
            violations.append(
                f"{path}:{i+1}: Direct Zenoh usage found in black-box tests.\n"
                "  Fix: Tests MUST use simulation.transport.publish() and subscribe() for compatibility.\n"
                f"  {ENTERPRISE_MANDATE} '# ZENOH_HACK_EXCEPTION: <reason>' at the top of the file."
            )
    return violations

def run_lint(targets: list[Path], excludes: list[str]) -> bool:
    """Executes the linting process. Returns True if passed, False if violations found."""
    all_violations = []

    for path in iter_target_files(targets, excludes, "*.py"):
        if "scripts/lints" in str(path):
            continue
        all_violations.extend(check_file(path))
        all_violations.extend(check_direct_zenoh_hacks(path))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False

    logger.info("✓ Python banned patterns check passed.")
    return True

def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Python banned patterns.")
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

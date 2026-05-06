#!/usr/bin/env python3
"""
Enforces Enterprise-Grade Rust coding standards and banned patterns.
This script ensures that the Rust simulation hot-path remains deterministic,
avoids unsafe threading practices, and adheres strictly to SOTA software quality.
Comment escapes are strongly discouraged.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import argparse
import logging
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent)) # noqa: TID251
from lint_utils import DEFAULT_EXCLUDES, ENTERPRISE_MANDATE, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)

# (Pattern, Message, Fix Suggestion, [Included Dirs], [Excluded Files/Dirs])
RUST_LINTS = [
    (
        r"thread::sleep",
        "Banned thread::sleep found.",
        f"Replace with condvar/channel. {ENTERPRISE_MANDATE} '// SLEEP_EXCEPTION: <reason>' inline.",
        ["hw/rust"],
        ["SLEEP_EXCEPTION"]
    ),
    (
        r"Mutex<",
        "Banned Mutex<T> in peripheral state.",
        f"Replace Mutex<T> with BqlGuarded<T> from virtmcu_qom::sync. {ENTERPRISE_MANDATE} '// MUTEX_EXCEPTION: <reason>'.",
        ["hw/rust/comms", "hw/rust/mcu", "hw/rust/observability"],
        ["Arc<Mutex", "MUTEX_EXCEPTION"]
    ),
    (
        r"Bql::lock\(\)|SafeSubscription",
        "Banned BQL usage found in async/comms paths.",
        f"Remove Bql::lock()/SafeSubscription, or use lock-free channels. {ENTERPRISE_MANDATE} '// BQL_EXCEPTION: <reason>'.",
        ["hw/rust/comms"],
        ["BQL_EXCEPTION"]
    ),
    (
        r"#\!\[no_std\]",
        "Misleading #![no_std] found.",
        f"Remove #![no_std]. {ENTERPRISE_MANDATE} '// NO_STD_EXCEPTION: <reason>' inline.",
        ["hw/rust"],
        ["NO_STD_EXCEPTION"]
    ),
    (
        r"to_ne_bytes|from_ne_bytes",
        "Banned to_ne_bytes/from_ne_bytes found.",
        f"Use to_le_bytes()/from_le_bytes() with a wire-order comment. {ENTERPRISE_MANDATE} '// NE_BYTES_EXCEPTION: <reason>'.",
        ["hw/rust"],
        ["NE_BYTES_EXCEPTION"]
    ),
    (
        r"rand::thread_rng\b",
        "Banned rand::thread_rng found.",
        f"Use seed_for_quantum() from transport-zenoh. {ENTERPRISE_MANDATE} '// RNG_EXCEPTION: <reason>'.",
        ["hw/rust"],
        ["RNG_EXCEPTION"]
    ),
    (
        r"#\[allow\(",
        "Banned #[allow(] found in production code.",
        f"Fix the underlying lint instead. {ENTERPRISE_MANDATE} '// ALLOW_EXCEPTION: <reason>' inline (for tests only).",
        ["hw/rust"],
        ["ALLOW_EXCEPTION", "target", "tests", "_generated.rs", "build.rs"]
    ),
]

def check_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    lines = content.splitlines()
    
    for pattern, msg, fix, inc_dirs, exc_list in RUST_LINTS:
        if not any(inc in str(path) for inc in inc_dirs):
            continue

        if any(exc in str(path) for exc in exc_list if "/" in exc or "." in exc):
            continue

        regex = re.compile(pattern)
        for i, line in enumerate(lines):
            if regex.search(line):
                if any(exc in line for exc in exc_list if "/" not in exc and "." not in exc):
                    continue
                violations.append(f"{path}:{i+1}: {msg}\n  Fix: {fix}\n  Line: {line.strip()}")

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
        
    logger.info("✓ Rust banned patterns check passed.")
    return True

def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Rust banned patterns.")
    parser.add_argument("targets", type=Path, nargs="*", default=[Path("hw/rust")], help="Target directories or files")
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

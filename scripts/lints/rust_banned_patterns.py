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

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lint_utils import (
    DEFAULT_EXCLUDES,
    ENTERPRISE_MANDATE,
    is_suppressed,
    iter_target_files,
    setup_lint_logging,
)

logger = logging.getLogger(__name__)

# (Rule Name, Pattern, Message, Fix Suggestion, [Included Dirs], [Legacy Keywords])
RUST_LINTS = [
    (
        "sleep",
        r"thread::sleep",
        "Banned thread::sleep found.",
        f"Replace with condvar/channel. {ENTERPRISE_MANDATE} '// virtmcu-allow: sleep reasoning=\"<reason>\"' inline.",
        ["hw/rust"],
        ["SLEEP_EXCEPTION"],
    ),
    (
        "mutex",
        r"Mutex<",
        "Banned Mutex<T> in peripheral state.",
        f"Replace Mutex<T> with BqlGuarded<T> from virtmcu_qom::sync. {ENTERPRISE_MANDATE} '// virtmcu-allow: mutex reasoning=\"<reason>\"'.",
        ["hw/rust/comms", "hw/rust/mcu", "hw/rust/observability"],
        ["Arc<Mutex", "MUTEX_EXCEPTION"],
    ),
    (
        "bql",
        r"Bql::lock\(\)|SafeSubscription",
        "Banned BQL usage found in async/comms paths.",
        f"Remove Bql::lock()/SafeSubscription, or use lock-free channels. {ENTERPRISE_MANDATE} '// virtmcu-allow: bql reasoning=\"<reason>\"'.",
        ["hw/rust/comms"],
        ["BQL_EXCEPTION"],
    ),
    (
        "no_std",
        r"#\!\[no_std\]",
        "Misleading #![no_std] found.",
        f"Remove #![no_std]. {ENTERPRISE_MANDATE} '// virtmcu-allow: no_std reasoning=\"<reason>\"' inline.",
        ["hw/rust"],
        ["NO_STD_EXCEPTION"],
    ),
    (
        "ne_bytes",
        r"to_ne_bytes|from_ne_bytes",
        "Banned to_ne_bytes/from_ne_bytes found.",
        f"Use to_le_bytes()/from_le_bytes() with a wire-order comment. {ENTERPRISE_MANDATE} '// virtmcu-allow: ne_bytes reasoning=\"<reason>\"'.",
        ["hw/rust"],
        ["NE_BYTES_EXCEPTION"],
    ),
    (
        "rng",
        r"rand::thread_rng\b",
        "Banned rand::thread_rng found.",
        f"Use seed_for_quantum() from transport-zenoh. {ENTERPRISE_MANDATE} '// virtmcu-allow: rng reasoning=\"<reason>\"'.",
        ["hw/rust"],
        ["RNG_EXCEPTION"],
    ),
    (
        "yield",
        r"std::thread::yield_now\b",
        "Banned thread::yield_now() found.",
        f"Replace with condvar/channel. {ENTERPRISE_MANDATE} '// virtmcu-allow: yield reasoning=\"<reason>\"' inline.",
        ["hw/rust"],
        ["YIELD_EXCEPTION"],
    ),
    (
        "allow",
        r"#\[allow\(",
        "Banned #[allow(] found in production code.",
        f"Fix the underlying lint instead. {ENTERPRISE_MANDATE} '// virtmcu-allow: allow reasoning=\"<reason>\"' inline (for tests only).",
        ["hw/rust"],
        ["ALLOW_EXCEPTION", "target", "tests", "_generated.rs", "build.rs"],
    ),
]

# (Rule Name, Pattern, Message, Fix Suggestion, [Included Dirs], [Legacy Keywords])
# These lints are checked against the entire file content (multi-line).
RUST_FILE_LINTS = [
    (
        "spinloop",
        r"(?:while|loop|for)[\s\S]{1,1000}?(?:attempts|retry|count)\s*[-+]=\s*1[\s\S]{1,1000}?(?:thread::sleep|yield_now|cvar\.wait_timeout)",
        "Banned bounded spinloop found (loop with counter mutation and sleep/yield/wait_timeout).",
        f"Use Condvar for waiting without iteration bounds (the Drain pattern) to avoid time-bomb UAF. {ENTERPRISE_MANDATE} '// virtmcu-allow: spinloop reasoning=\"<reason>\"'.",
        ["hw/rust"],
        ["SPINLOOP_EXCEPTION"],
    ),
    (
        "drop_bound",
        r"impl\s+Drop\s+for[\s\S]{1,500}?(?:while|loop|for)[\s\S]{1,200}?(?:attempts|retry|count)\s*<",
        "Banned bounded loop in Drop implementation.",
        f"Drop implementations must use the Drain pattern (while active > 0 {{ cvar.wait(guard) }}) and NOT use bounded iteration limits to avoid time-bomb UAF. {ENTERPRISE_MANDATE} '// virtmcu-allow: drop_bound reasoning=\"<reason>\"'.",
        ["hw/rust"],
        ["DROP_BOUND_EXCEPTION"],
    ),
]


def check_single_slot_callbacks(path: Path, content: str) -> list[str]:
    violations = []

    # Only run on hw/rust
    if "hw/rust" not in str(path):
        return violations

    rule_name = "callback"
    # Find type aliases and structs containing function pointers
    banned_types = set()

    type_alias_pattern = re.compile(r"type\s+(\w+)\s*=\s*(?:Option\s*<\s*)?[^;={]*?fn\s*\(", re.MULTILINE)
    for match in type_alias_pattern.finditer(content):
        banned_types.add(match.group(1))

    struct_pattern = re.compile(r"struct\s+(\w+)\s*\{[^}]*?fn\s*\([^}]*?\}", re.MULTILINE)
    for match in struct_pattern.finditer(content):
        banned_types.add(match.group(1))

    # Check all static mut declarations
    static_mut_pattern = re.compile(r"static\s+mut\s+(\w+)\s*:\s*([^;=]+)", re.MULTILINE)
    for match in static_mut_pattern.finditer(content):
        type_str = match.group(2).strip()
        # Determine the full declaration block (up to the semicolon + rest of that line)
        start_idx = match.start()
        semi_idx = content.find(";", start_idx)
        if semi_idx == -1:
            semi_idx = len(content)
        eol_idx = content.find("\n", semi_idx)
        if eol_idx == -1:
            eol_idx = len(content)

        decl_block = content[start_idx:eol_idx]
        if is_suppressed(decl_block, rule_name):
            continue

        is_banned = False

        # 1. Check if type contains banned type
        for b_type in banned_types:
            if re.search(rf"\b{b_type}\b", type_str):
                is_banned = True
                break

        # 2. Check inline inline regex
        if not is_banned and (
            re.search(r"^(?!\[)(?:Option\s*<\s*)?[^;={]*?fn\s*\(", type_str) or re.search(r"\(.*?\bfn\s*\(", type_str)
        ):
            is_banned = True

        if is_banned:
            line_no = content.count("\n", 0, match.start()) + 1
            msg = "Banned single-slot global callback found."
            fix = f"Use an array of hooks (e.g., [Option<fn()>; 8]) or pass via Dependency Injection to prevent silent plugin clobbering. {ENTERPRISE_MANDATE} '// virtmcu-allow: {rule_name} reasoning=\"<reason>\"'."
            violations.append(f"{path}:{line_no}: {msg}\n  Fix: {fix}")

    return violations


def check_file(path: Path, force_all: bool = False) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return []

    lines = content.splitlines()

    violations.extend(check_single_slot_callbacks(path, content))

    # Line-based lints
    for rule_name, pattern, msg, fix, inc_dirs, exc_list in RUST_LINTS:
        if not force_all and not any(inc in str(path) for inc in inc_dirs):
            continue

        # File-level exclusions (paths or file patterns in exc_list)
        if any(exc in str(path) for exc in exc_list if "/" in exc or "." in exc):
            continue

        regex = re.compile(pattern)
        for i, line in enumerate(lines):
            if regex.search(line):
                # Line-level suppression (keywords in exc_list)
                if is_suppressed(line, rule_name):
                    continue
                violations.append(f"{path}:{i + 1}: {msg}\n  Fix: {fix}\n  Line: {line.strip()}")

    # File-based lints (multi-line)
    for rule_name, pattern, msg, fix, inc_dirs, exc_list in RUST_FILE_LINTS:
        if not force_all and not any(inc in str(path) for inc in inc_dirs):
            continue

        if any(exc in str(path) for exc in exc_list if "/" in exc or "." in exc):
            continue

        if is_suppressed(content, rule_name):
            continue

        regex = re.compile(pattern, re.MULTILINE)
        for match in regex.finditer(content):
            line_no = content.count("\n", 0, match.start()) + 1
            violations.append(f"{path}:{line_no}: {msg}\n  Fix: {fix}")

    return violations


def run_lint(targets: list[Path], excludes: list[str], force_all: bool = False) -> bool:
    """Executes the linting process. Returns True if passed, False if violations found."""
    all_violations = []

    for path in iter_target_files(targets, excludes, "*.rs"):
        all_violations.extend(check_file(path, force_all))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False

    logger.info("✓ Rust banned patterns check passed.")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Rust banned patterns.")
    parser.add_argument("targets", type=Path, nargs="*", default=[Path("hw/rust")], help="Target directories or files")
    parser.add_argument("--exclude", nargs="*", default=DEFAULT_EXCLUDES, help="Directories to exclude")
    parser.add_argument("--force-all", action="store_true", help="Apply all lints to all files (ignore inc_dirs)")
    args = parser.parse_args()

    setup_lint_logging()
    success = run_lint(args.targets, args.exclude, args.force_all)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()

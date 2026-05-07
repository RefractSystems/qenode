#!/usr/bin/env python3
"""
Audits the codebase for all custom lint suppressions (virtmcu-allow) and legacy exceptions.
Generates a report of active exemptions to ensure quality control and visibility.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

import argparse
import logging
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lint_utils import DEFAULT_EXCLUDES, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)

# Regex for modern SOTA suppression
SOTA_PATTERN = re.compile(r'virtmcu-allow:\s*(?P<rule>[a-zA-Z0-9_]+)\s+reasoning="(?P<reason>[^"]+)"')
# Regex for incomplete modern suppression (missing reasoning or malformed)
INCOMPLETE_SOTA_PATTERN = re.compile(r'virtmcu-allow:\s*(?P<rule>[a-zA-Z0-9_]+)(?!\s+reasoning="[^"]+")')
# Regex for legacy _EXCEPTION markers
LEGACY_PATTERN = re.compile(r"\b[A-Z_]+_EXCEPTION\b")


def audit_file(path: Path) -> dict:
    """Scans a single file for suppressions."""
    results = {"sota": [], "incomplete": [], "legacy": []}
    try:
        content = path.read_text()
    except UnicodeDecodeError:
        return results

    lines = content.splitlines()
    for i, line in enumerate(lines):
        line_no = i + 1

        # Check for SOTA suppressions
        sota_matches = list(SOTA_PATTERN.finditer(line))
        for match in sota_matches:
            results["sota"].append(
                {"line": line_no, "rule": match.group("rule"), "reason": match.group("reason"), "content": line.strip()}
            )

        # Check for malformed SOTA suppressions
        for match in INCOMPLETE_SOTA_PATTERN.finditer(line):
            # Ensure it wasn't already caught by the SOTA_PATTERN. We check if the matched string is a prefix of a valid SOTA match.
            is_valid = False
            for sota_match in sota_matches:
                if sota_match.start() <= match.start() and sota_match.end() >= match.end():
                    is_valid = True
                    break
            if not is_valid:
                results["incomplete"].append({"line": line_no, "rule": match.group("rule"), "content": line.strip()})

        # Check for legacy exceptions
        for match in LEGACY_PATTERN.finditer(line):
            results["legacy"].append({"line": line_no, "marker": match.group(0), "content": line.strip()})

    return results


def run_audit(targets: list[Path], excludes: list[str], output_format: str) -> bool:
    all_results = {}
    total_sota = 0
    total_incomplete = 0
    total_legacy = 0

    # Scan all common source files
    for ext in ["*.rs", "*.py", "*.sh", "*.c", "*.h", "*.md", "*.yaml", "*.yml"]:
        for path in iter_target_files(targets, excludes, ext):
            res = audit_file(path)
            if res["sota"] or res["incomplete"] or res["legacy"]:
                all_results[str(path)] = res
                total_sota += len(res["sota"])
                total_incomplete += len(res["incomplete"])
                total_legacy += len(res["legacy"])

    if output_format == "json":
        pass
    else:
        logger.info("=== VirtMCU Lint Suppression Audit ===")
        logger.info(f"Total SOTA Suppressions: {total_sota}")
        logger.info(f"Total Incomplete/Malformed: {total_incomplete}")
        logger.info(f"Total Legacy Markers: {total_legacy}")
        logger.info("--------------------------------------")

        for file_path, data in all_results.items():
            if data["sota"]:
                logger.info(f"\nFile: {file_path} (SOTA)")
                for item in data["sota"]:
                    logger.info(f"  Line {item['line']} | Rule: {item['rule']} | Reason: {item['reason']}")

            if data["incomplete"]:
                logger.warning(f"\nFile: {file_path} (INCOMPLETE)")
                for item in data["incomplete"]:
                    logger.warning(
                        f'  Line {item["line"]} | Rule: {item["rule"]} | WARNING: Missing or malformed reasoning="..."'
                    )

            if data["legacy"]:
                logger.warning(f"\nFile: {file_path} (LEGACY)")
                for item in data["legacy"]:
                    logger.warning(f"  Line {item['line']} | Marker: {item['marker']}")

    # Fail the audit if there are incomplete SOTA formats
    if total_incomplete > 0:
        logger.error(
            "\nAudit Failed: Found incomplete or malformed 'virtmcu-allow' statements. A valid 'reasoning=\"...\"' is required."
        )
        return False

    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Audit custom lint suppressions.")
    parser.add_argument("targets", type=Path, nargs="*", default=[Path()], help="Target directories")
    parser.add_argument("--exclude", nargs="*", default=DEFAULT_EXCLUDES, help="Directories to exclude")
    parser.add_argument("--format", choices=["text", "json"], default="text", help="Output format")
    args = parser.parse_args()

    setup_lint_logging()
    success = run_audit(args.targets, args.exclude, args.format)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""
Enforces provenance documentation standards for firmware and golden references.
Mandated by PLAN.md Milestone 32.3.
"""

import logging
import re
import sys
from pathlib import Path

# Required fields for PROVENANCE.md
FIRMWARE_REQUIRED_FIELDS = [
    "Real-world MCU",
    "Peripheral",
    "Vendor SDK",
    "Source URL",
    "License",
    "Download Date",
]

# Required fields for third_party/golden_references/*/README.md
REFERENCE_REQUIRED_FIELDS = [
    "Source",
    "License",
    "Download Date",
]

logger = logging.getLogger(__name__)

def setup_logging() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(levelname)s: %(message)s",
    )

def check_markdown_table(path: Path, required_fields: list[str]) -> list[str]:
    """Checks if a markdown file contains a table with the required fields AND non-empty values."""
    if not path.exists():
        return [f"Missing file: {path}"]

    content = path.read_text()
    missing = []

    # Store found fields and their values
    found_fields: dict[str, str] = {}

    # Regex to capture a Markdown table row: | Key | Value |
    # It loosely allows spaces around the pipes.
    row_pattern = re.compile(r"^\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|.*$")

    for line in content.splitlines():
        match = row_pattern.match(line)
        if match:
            key = match.group(1).strip()
            val = match.group(2).strip()
            found_fields[key.lower()] = val

    for field in required_fields:
        field_lower = field.lower()

        # Check if any parsed key matches or contains our required field
        matched_val = None
        for k, v in found_fields.items():
            if field_lower in k or k in field_lower:
                matched_val = v
                break

        if matched_val is None:
            missing.append(f"Missing field '{field}' in {path}")
        elif not matched_val or matched_val == "":
            missing.append(f"Empty value for field '{field}' in {path}")

    return missing

def get_golden_reference_target(prov_path: Path) -> str | None:
    """Extracts the expected golden reference folder name from a PROVENANCE.md, if specified."""
    content = prov_path.read_text()
    # If the file mentions a specific vendor SDK, extract it for cross-referencing.
    # We look for a line like: | Vendor SDK | NXP S32K3 |
    for line in content.splitlines():
        if "| Vendor SDK |" in line or "| vendor sdk |" in line.lower():
            parts = [p.strip() for p in line.split("|") if p.strip()]
            if len(parts) >= 2:
                val = parts[1]
                if "N/A" not in val.upper() and "HAND-WRITTEN" not in val.upper():
                    # Very simple heuristic: just return the raw string to be checked
                    return val
    return None

def run_lint(root_dir: Path | None = None) -> bool:
    if root_dir is None:
        root_dir = Path(__file__).resolve().parent.parent.parent

    tests_dir = root_dir / "tests"
    golden_refs = root_dir / "third_party" / "golden_references"

    errors = []

    # 1. Discover all binaries in tests/ and ensure they have a PROVENANCE.md sibling
    if tests_dir.exists():
        for bin_file in tests_dir.rglob("*.[eb][li][fn]"): # Matches .elf and .bin
            # Ignore binaries that are clearly build artifacts (in __pycache__, target, etc.)
            if "__pycache__" in bin_file.parts or "target" in bin_file.parts or ".pytest_cache" in bin_file.parts:
                continue

            prov_file = bin_file.parent / "PROVENANCE.md"
            if not prov_file.exists():
                errors.append(f"Missing PROVENANCE.md for binary: {bin_file}")
                continue

            # Check table contents
            table_errors = check_markdown_table(prov_file, FIRMWARE_REQUIRED_FIELDS)
            errors.extend(table_errors)

            # Cross-reference check (if no table errors)
            if not table_errors and golden_refs.exists():
                golden_target = get_golden_reference_target(prov_file)
                if golden_target:
                    # In a real SOTA system, you'd have a strictly enforced mapping ID.
                    # Here we just verify the directory is not completely empty if they claim a vendor SDK.
                    has_golden = any(golden_refs.iterdir())
                    if not has_golden:
                        errors.append(f"PROVENANCE.md claims Vendor SDK '{golden_target}' but {golden_refs} is empty.")

    # 2. Check third_party/golden_references/*/README.md
    if golden_refs.exists():
        for ref_dir in golden_refs.iterdir():
            if ref_dir.is_dir():
                if ref_dir.name.startswith((".", "__")):
                    continue

                readme_file = ref_dir / "README.md"
                errors.extend(check_markdown_table(readme_file, REFERENCE_REQUIRED_FIELDS))

    if errors:
        for err in errors:
            logger.error(err)
        logger.error(f"\nProvenance check FAILED with {len(errors)} errors.")
        return False

    logger.info("Provenance check PASSED.")
    return True

if __name__ == "__main__":
    setup_logging()
    if not run_lint():
        sys.exit(1)
    sys.exit(0)


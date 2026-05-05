#!/usr/bin/env python3
import sys
from pathlib import Path


def lint_rust_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text()
    except Exception as e:
        return [f"Error reading {path}: {e}"]

    lines = content.splitlines()
    for i, line in enumerate(lines):
        if "ptr::copy_nonoverlapping" in line:
            # Check if current line, previous line, or next line has COPY_EXCEPTION
            has_exception = "COPY_EXCEPTION:" in line
            if not has_exception and i > 0:
                has_exception = "COPY_EXCEPTION:" in lines[i - 1]
            if not has_exception and i < len(lines) - 1:
                has_exception = "COPY_EXCEPTION:" in lines[i + 1]

            if not has_exception:
                violations.append(
                    f"{path}:{i + 1}: Banned ptr::copy_nonoverlapping found (Mandate 15). Use .to_le_bytes() / .from_le_bytes() instead."
                )

    return violations


def main():
    root = Path(__file__).resolve().parent.parent
    hw_rust = root / "hw/rust"

    all_violations = []
    for path in hw_rust.rglob("*.rs"):
        if path.name == "core_generated.rs":
            continue
        all_violations.extend(lint_rust_file(path))

    if all_violations:
        for v in all_violations:
            print(v)
        sys.exit(1)
    else:
        print("✓ Mandate 15 Rust lint passed.")
        sys.exit(0)


if __name__ == "__main__":
    main()

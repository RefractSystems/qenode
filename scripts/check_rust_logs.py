#!/usr/bin/env python3
import os
import re
import subprocess


def check_rust_prints():
    hw_rust_dir = "hw/rust"
    if not os.path.exists(hw_rust_dir):
        return

    # Find all println! and eprintln! calls
    try:
        output = subprocess.check_output(
            ["grep", "-rnE", r"(println!|eprintln!)\(", hw_rust_dir, "--include=*.rs", "--exclude-dir=target"],
            text=True,
        )
    except subprocess.CalledProcessError as e:
        if e.returncode == 1:  # No matches
            return
        raise

    violations = []
    for line in output.splitlines():
        if not line:
            continue
        parts = line.split(":", 2)
        if len(parts) < 3:
            continue
        file_path, line_num, content = parts
        line_num = int(line_num)

        # Skip Miri eprintln! (handled in telemetry.rs)
        if "eprintln!" in content and "Miri" in content:
            continue

        # Check if same line has PRINT_EXCEPTION
        if "PRINT_EXCEPTION" in content:
            continue

        # Check surrounding lines (up to 5 lines after)
        with open(file_path, "r") as f:
            lines = f.readlines()
            found = False
            for i in range(line_num, min(line_num + 5, len(lines))):
                if "PRINT_EXCEPTION" in lines[i]:
                    found = True
                    break
            if found:
                continue

        violations.append(line)

    if violations:
        print("❌ ERROR: Banned println!/eprintln! found in hw/rust/:")
        for v in violations:
            print(v)
        print("  Fix: replace with sim_info!/sim_err!, or add // PRINT_EXCEPTION: <reason> inline.")
        exit(1)
    else:
        print("✓ No banned println!/eprintln! found.")


if __name__ == "__main__":
    check_rust_prints()

#!/usr/bin/env python3
import subprocess
import sys
from pathlib import Path

# Mandatory symbols that every VirtMCU plugin must export
REQUIRED_SYMBOLS = {
    "hw-virtmcu-clock.so": ["clock_cpu_halt_cb"],
    # Add other plugins and their required symbols here
}

def check_symbols(so_path: Path, required: list[str]) -> bool:
    print(f"Checking {so_path.name} for required FFI symbols...")
    try:
        # -D/--dynamic: Look at the dynamic symbol table
        # -G/--external-only: Only look at external symbols
        result = subprocess.run(
            ["nm", "-D", str(so_path)],
            capture_output=True,
            text=True,
            check=True
        )

        exported_symbols = [line.split()[-1] for line in result.stdout.splitlines() if " T " in line]

        missing = [s for s in required if s not in exported_symbols]
        if missing:
            print(f"❌ ERROR: {so_path.name} is missing mandatory unmangled symbols: {missing}")
            print("   Ensure these are marked with #[no_mangle] extern \"C\" in Rust.")
            return False

        print(f"✅ {so_path.name}: All symbols found.")
        return True
    except subprocess.CalledProcessError as e:
        print(f"❌ ERROR: Failed to run 'nm' on {so_path}: {e}")
        return False

def main():
    build_dir = Path("third_party/qemu/build-virtmcu")
    if not build_dir.exists():
        print(f"Build directory {build_dir} not found. Skipping export check.")
        return 0

    success = True
    for so_name, symbols in REQUIRED_SYMBOLS.items():
        so_path = build_dir / so_name
        if so_path.exists():
            if not check_symbols(so_path, symbols):
                success = False
        else:
            # Not all plugins might be built, that's fine
            continue

    return 0 if success else 1

if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
import logging
import shutil
import subprocess
import sys
from pathlib import Path


def _get_cmd(cmd: str) -> str:
    p = shutil.which(cmd)
    assert p is not None, f"Command {cmd} not found"
    return p


logger = logging.getLogger(__name__)

# Mandatory symbols that every VirtMCU plugin must export
REQUIRED_SYMBOLS = {
    "hw-virtmcu-clock.so": ["clock_cpu_halt_cb"],
    # Add other plugins and their required symbols here
}

# Mandatory symbols that the main QEMU executable MUST export dynamically to plugins
QEMU_REQUIRED_EXPORTS = [
    "virtmcu_cpu_set_tcg_hook",
    "virtmcu_cpu_set_halt_hook",
    "virtmcu_set_irq_hook",
    "virtmcu_kick_first_cpu_for_quantum",
    "virtmcu_vcpu_should_yield",
    "virtmcu_is_bql_locked",
    "virtmcu_safe_bql_unlock",
    "virtmcu_safe_bql_lock",
    "virtmcu_safe_bql_force_unlock",
    "virtmcu_safe_bql_force_lock",
]


def check_symbols(so_path: Path, required: list[str], is_executable: bool = False) -> bool:
    target_type = "executable" if is_executable else "plugin"
    logger.info(f"Checking {target_type} {so_path.name} for required FFI symbols...")
    try:
        # Prefer llvm-nm if available to handle LTO/bitcode better, fallback to nm
        nm_tool = "llvm-nm"
        if subprocess.run([_get_cmd("which"), nm_tool], capture_output=True).returncode != 0:
            nm_tool = "nm"

        # -D/--dynamic: Look at the dynamic symbol table
        result = subprocess.run([nm_tool, "-D", str(so_path)], capture_output=True, text=True, check=False)
        if result.returncode != 0:
            logger.info(f"❌ ERROR: nm failed with return code {result.returncode}")
            logger.info(f"   STDOUT: {result.stdout}")
            logger.info(f"   STDERR: {result.stderr}")
            return False

        # In executables, some symbols might be B (BSS) rather than T (Text) if they are just pointers.
        exported_symbols = [
            line.split()[-1]
            for line in result.stdout.splitlines()
            if any(t in line for t in [" T ", " B ", " D ", " W "])
        ]

        missing = [s for s in required if s not in exported_symbols]
        if missing:
            logger.info(f"❌ ERROR: {so_path.name} is missing mandatory unmangled symbols: {missing}")
            if not is_executable:
                logger.info('   Ensure these are marked with #[no_mangle] extern "C" in Rust.')
            else:
                logger.info('   Ensure these are marked with __attribute__((visibility("default"))) in QEMU C code.')
            return False

        logger.info(f"✅ {so_path.name}: All symbols found.")
        return True
    except subprocess.CalledProcessError as e:
        logger.info(f"❌ ERROR: Failed to run 'nm' on {so_path}: {e}")
        return False


def main() -> int:
    build_dir = Path("third_party/qemu/build-virtmcu")
    if not build_dir.exists():
        logger.info(f"Build directory {build_dir} not found. Skipping export check.")
        return 0

    success = True

    # Check main executable
    qemu_bin = build_dir / "qemu-system-arm"
    if qemu_bin.exists() and not check_symbols(qemu_bin, QEMU_REQUIRED_EXPORTS, is_executable=True):
        success = False

    # Check plugins
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
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    sys.exit(main())

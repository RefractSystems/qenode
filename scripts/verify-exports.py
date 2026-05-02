#!/usr/bin/env python3
import logging
import shutil
import subprocess
import sys
from pathlib import Path

# Try to import WORKSPACE_DIR from our internal environment helper
try:
    from tools.testing.env import WORKSPACE_DIR
except ImportError:
    WORKSPACE_DIR = Path(__file__).resolve().parent.parent


def _get_nm_tool() -> str:
    # Prefer llvm-nm if available to handle LTO/bitcode better, fallback to nm
    for tool in ["llvm-nm", "nm"]:
        if shutil.which(tool):
            return tool
    raise RuntimeError("Neither 'llvm-nm' nor 'nm' found in PATH")


logger = logging.getLogger(__name__)

# Mandatory symbols that every VirtMCU plugin must export
REQUIRED_SYMBOLS = {
    "hw-virtmcu-clock.so": ["clock_cpu_halt_cb"],
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
    logger.info(f"Checking {target_type} {so_path} for required FFI symbols...")

    if not so_path.exists():
        logger.info(f"⚠️  {target_type.capitalize()} {so_path} not found. Skipping.")
        return True

    try:
        nm_tool = _get_nm_tool()
        # -D/--dynamic: Look at the dynamic symbol table
        result = subprocess.run([nm_tool, "-D", str(so_path)], capture_output=True, text=True, check=False)
        if result.returncode != 0:
            logger.info(f"❌ ERROR: {nm_tool} -D failed with return code {result.returncode} for {so_path}")
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
    except (OSError, RuntimeError) as e:
        logger.info(f"❌ ERROR: Unexpected error while checking {so_path}: {e}")
        return False


def main() -> int:
    # Check multiple possible build locations
    build_dirs = [
        WORKSPACE_DIR / "third_party/qemu/build-virtmcu",
        WORKSPACE_DIR / "third_party/qemu/build-virtmcu-asan",
    ]

    success = True
    found_any = False

    for build_dir in build_dirs:
        if not build_dir.exists():
            continue

        found_any = True
        # Check main executable
        qemu_bin = build_dir / "qemu-system-arm"
        if not check_symbols(qemu_bin, QEMU_REQUIRED_EXPORTS, is_executable=True):
            success = False

        # Check plugins
        for so_name, symbols in REQUIRED_SYMBOLS.items():
            so_path = build_dir / so_name
            if not check_symbols(so_path, symbols):
                success = False

    if not found_any:
        logger.info("Build directory not found. Skipping export check.")
        return 0

    return 0 if success else 1


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    sys.exit(main())

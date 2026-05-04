"""
Reads a base DTS file, performs string replacements, and compiles it into a DTB.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def inspect_dtb(dtb_path: Path | str) -> str:
    """
    Invokes dtc to decompile a DTB into a DTS string for introspection.
    """
    dtb_path = Path(dtb_path)
    dtc_cmd = shutil.which("dtc") or "dtc"
    try:
        res = subprocess.run(
            [dtc_cmd, "-I", "dtb", "-O", "dts", str(dtb_path)],
            check=True,
            capture_output=True,
            text=True,
        )
        return res.stdout
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"dtc failed: {e.stderr}") from e


def compile_repl(repl_path: Path | str, out_dtb: Path | str) -> Path:
    """
    Invokes repl2qemu to compile a .repl file into a DTB.
    """
    repl_path = Path(repl_path)
    out_dtb = Path(out_dtb)
    python_cmd = sys.executable
    try:
        subprocess.run(
            [python_cmd, "-m", "tools.repl2qemu", str(repl_path), "--out-dtb", str(out_dtb)],
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"repl2qemu failed: {e.stderr}") from e
    return out_dtb


def compile_yaml(yaml_path: Path | str, out_dtb: Path | str, out_cli: Path | str | None = None) -> Path:
    """
    Invokes yaml2qemu to compile a World Topology YAML into a DTB.
    """
    yaml_path = Path(yaml_path)
    out_dtb = Path(out_dtb)
    python_cmd = sys.executable
    cmd = [python_cmd, "-m", "tools.yaml2qemu", str(yaml_path), "--out-dtb", str(out_dtb)]
    if out_cli:
        cmd.extend(["--out-cli", str(out_cli)])

    try:
        subprocess.run(
            cmd,
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"yaml2qemu failed: {e.stderr}") from e
    return out_dtb


def compile_dtb(base_dts: Path | str, replacements: dict[str, str], out_dtb: Path | str) -> Path:

    base_dts = Path(base_dts)
    out_dtb = Path(out_dtb)

    content = base_dts.read_text()
    for k, v in replacements.items():
        content = content.replace(k, v)

    with tempfile.NamedTemporaryFile(mode="w", suffix=".dts", delete=False) as tf:
        tf.write(content)
        tmp_dts = tf.name

    try:
        dtc_cmd = shutil.which("dtc") or "dtc"
        subprocess.run(
            [dtc_cmd, "-I", "dts", "-O", "dtb", "-o", str(out_dtb), tmp_dts], check=True, capture_output=True, text=True
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"dtc failed: {e.stderr}") from e
    finally:
        if Path(tmp_dts).exists():
            Path(tmp_dts).unlink()

    return out_dtb


def compile_firmware(
    source_files: list[Path | str],
    out_elf: Path | str,
    linker_script: Path | str | None = None,
    cpu: str = "cortex-a15",
) -> Path:
    """
    Compiles a list of source files (C or Assembly) into a bare-metal ELF.
    """
    out_elf = Path(out_elf)
    gcc_cmd = shutil.which("arm-none-eabi-gcc") or "arm-none-eabi-gcc"
    cmd = [gcc_cmd, f"-mcpu={cpu}", "-nostdlib"]

    if linker_script:
        cmd.extend(["-T", str(linker_script)])

    for src in source_files:
        cmd.append(str(src))

    cmd.extend(["-o", str(out_elf)])

    try:
        subprocess.run(cmd, check=True, capture_output=True, text=True)
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"arm-none-eabi-gcc failed: {e.stderr}") from e

    return out_elf


def validate_dtb(dtb_path: Path | str) -> bool:
    """
    Invokes dt-validate to check a DTB against schemas.
    Returns True if valid, False if dt-validate is missing.
    Raises RuntimeError if validation fails.
    """
    dtb_path = Path(dtb_path)
    dt_validate = shutil.which("dt-validate")
    if not dt_validate:
        return False
    try:
        subprocess.run([dt_validate, str(dtb_path)], check=True, capture_output=True, text=True)
        return True
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"dt-validate failed: {e.stderr}") from e


def compile_c_snippet(
    snippet: str, out_dir: Path | str, linker_script: Path | str | None = None, filename: str = "snippet.c"
) -> Path:
    """
    Convenience method to write a C snippet to disk and compile it to an ELF.
    """
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    src_file = out_dir / filename
    src_file.write_text(snippet)

    out_elf = out_dir / (src_file.stem + ".elf")
    compile_firmware([src_file], out_elf, linker_script)
    return out_elf

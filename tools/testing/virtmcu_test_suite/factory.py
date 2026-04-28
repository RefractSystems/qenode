import subprocess
import tempfile
from pathlib import Path


def compile_dtb(base_dts: Path | str, replacements: dict[str, str], out_dtb: Path | str) -> Path:
    """
    Reads a base DTS file, performs string replacements, and compiles it into a DTB.
    """
    base_dts = Path(base_dts)
    out_dtb = Path(out_dtb)

    content = base_dts.read_text()
    for k, v in replacements.items():
        content = content.replace(k, v)

    with tempfile.NamedTemporaryFile(mode="w", suffix=".dts", delete=False) as tf:
        tf.write(content)
        tmp_dts = tf.name

    try:
        subprocess.run(
            ["dtc", "-I", "dts", "-O", "dtb", "-o", str(out_dtb), tmp_dts],
            check=True,
            capture_output=True,
            text=True
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"dtc failed: {e.stderr}") from e
    finally:
        if Path(tmp_dts).exists():
            Path(tmp_dts).unlink()

    return out_dtb


def compile_firmware(source_files: list[Path | str], out_elf: Path | str, linker_script: Path | str | None = None, cpu: str = "cortex-a15") -> Path:
    """
    Compiles a list of source files (C or Assembly) into a bare-metal ELF.
    """
    out_elf = Path(out_elf)
    cmd = ["arm-none-eabi-gcc", f"-mcpu={cpu}", "-nostdlib"]

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


def compile_c_snippet(snippet: str, out_dir: Path | str, linker_script: Path | str | None = None, filename: str = "snippet.c") -> Path:
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

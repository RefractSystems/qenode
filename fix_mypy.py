import re
from pathlib import Path


def process_file(filepath):
    path = Path(filepath)
    if not path.exists():
        return
    content = path.read_text()

    # Fix `while curr != "/"` -> `while str(curr) != "/"`
    content = re.sub(
        r'while\s+curr\s*!=\s*"/"\s+and\s+not\s+Path\(Path\(curr\)\.exists\(\)\s*/\s*"tools"\):',
        r'while str(curr) != "/" and not (curr / "tools").exists():',
        content,
    )
    content = re.sub(
        r'while\s+curr\s*!=\s*"/"\s+and\s+not\s+Path\(Path\(curr\)\.exists\(\)\s*/\s*"scripts"\):',
        r'while str(curr) != "/" and not (curr / "scripts").exists():',
        content,
    )

    # Fix QmpBridge connect
    content = re.sub(r"bridge\.connect\((Path[^,)]+),", r"bridge.connect(str(\1),", content)
    content = re.sub(r"QmpBridge\.connect\((Path[^,)]+),", r"QmpBridge.connect(str(\1),", content)

    # Fix subprocess *cmd list type issues
    content = re.sub(
        r'cmd = \[run_script, "--dtb", Path\(dtb_path\)\.resolve\(\)\]',
        r'cmd: list[str] = [str(run_script), "--dtb", str(Path(dtb_path).resolve())]',
        content,
    )
    content = re.sub(
        r'cmd\.extend\(\["--kernel", Path\(kernel_path\)\.resolve\(\)\]\)',
        r'cmd.extend(["--kernel", str(Path(kernel_path).resolve())])',
        content,
    )

    # Replace zenoh-clock check
    content = re.sub(r'if\s+"zenoh-clock"\s+in\s+arg:', r'if "zenoh-clock" in str(arg):', content)

    path.write_text(content)


for f in ["tests/conftest.py", "tools/testing/conftest.py", "tests/test_phase7.py"]:
    process_file(f)

# Fix test_flexray.py specifically
flexray = Path("tests/test_flexray.py")
if flexray.exists():
    c = flexray.read_text()
    c = c.replace(
        'with Path((Path(tmpdir).open() / "firmware.S"), "w") as fw_file:',
        'with (Path(tmpdir) / "firmware.S").open("w") as fw_file:',
    )
    c = c.replace('dtb_path = (Path(tmpdir) / "platform.dtb")', 'dtb_path = str(Path(tmpdir) / "platform.dtb")')
    c = c.replace(
        'dtb_path = (Path(phase27_dir) / "platform.dtb")', 'dtb_path = str(Path(phase27_dir) / "platform.dtb")'
    )
    c = c.replace(
        'kernel_path = (Path(phase27_dir) / "firmware.elf")', 'kernel_path = str(Path(phase27_dir) / "firmware.elf")'
    )
    # Fix YAML generation call
    c = c.replace(
        '"tools.yaml2qemu", (Path(tmpdir) / "platform.yaml"), "--out-dtb", dtb_path]',
        '"tools.yaml2qemu", str(Path(tmpdir) / "platform.yaml"), "--out-dtb", str(dtb_path)]',
    )
    flexray.write_text(c)

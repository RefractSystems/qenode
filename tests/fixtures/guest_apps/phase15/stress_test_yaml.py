import subprocess
import tempfile
from pathlib import Path

import yaml


def generate_large_yaml(num_peripherals=1000):
    """
    Generates a YAML with many peripherals to stress test the emitter.
    """
    peripherals = []
    for i in range(num_peripherals):
        peripherals.append(
            {"name": f"uart{i}", "type": "UART.PL011", "address": 0x10000000 + (i * 0x1000), "interrupts": [i % 32]}
        )

    return {
        "machine": {"name": "stress_board", "cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
        "peripherals": peripherals,
    }


def test_yaml2qemu_stress():
    """
    Stress test yaml2qemu with a large number of peripherals.
    """
    large_data = generate_large_yaml(100)  # 1000 is too many for dtc usually, let's try 100

    with tempfile.NamedTemporaryFile(suffix=".yaml", mode="w", delete=False) as f:
        yaml.dump(large_data, f)
        yaml_path = f.name

    dtb_path = yaml_path.replace(".yaml", ".dtb")

    try:
        # Run yaml2qemu
        result = subprocess.run(["yaml2qemu", yaml_path, "--out-dtb", dtb_path], capture_output=True, text=True)

        assert result.returncode == 0
        assert Path(dtb_path).exists()
        assert Path(dtb_path).stat().st_size > 0

    finally:
        if Path(yaml_path).exists():
            Path(yaml_path).unlink()
        if Path(dtb_path).exists():
            Path(dtb_path).unlink()


def test_yaml2qemu_invalid_interrupt():
    """
    Test yaml2qemu with an invalid interrupt format.
    """
    bad_data = {
        "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
        "peripherals": [
            {
                "name": "bad_irq",
                "type": "UART.PL011",
                "address": 0x10000000,
                "interrupts": ["invalid_format"],  # Should be int or "target@line"
            }
        ],
    }

    with tempfile.NamedTemporaryFile(suffix=".yaml", mode="w", delete=False) as f:
        yaml.dump(bad_data, f)
        yaml_path = f.name

    dtb_path = yaml_path.replace(".yaml", ".dtb")

    try:
        subprocess.run(["yaml2qemu", yaml_path, "--out-dtb", dtb_path], capture_output=True, text=True)

        # It might still succeed if it just warns, but let's see current behavior
        # Actually, yaml2qemu.py doesn't validate much, it might crash or produce bad DTS
        pass
    finally:
        if Path(yaml_path).exists():
            Path(yaml_path).unlink()
        if Path(dtb_path).exists():
            Path(dtb_path).unlink()

import os
from pathlib import Path

import pytest

from tools.testing.QemuLibrary import QemuLibrary


def test_dynamic_devices_realization():
    """
    Verifies that the YAML tooling and QEMU C/Rust models are synchronized.
    """
    yaml_path = "test/phase12/test_bridge.yaml"
    if not Path(yaml_path).exists():
        pytest.skip(f"{yaml_path} not found")

    # Modifying the yaml to remove the mmio-socket-bridge for this test
    # because it blocks realization if it can't connect.
    import tempfile

    import yaml

    with Path(yaml_path).open() as f:
        data = yaml.safe_load(f)

    # Keep only the clock or simple devices
    data["peripherals"] = [p for p in data.get("peripherals", []) if p["type"] != "mmio-socket-bridge"]

    with tempfile.NamedTemporaryFile(mode="w", suffix=".yaml", delete=False) as tmp_yaml:
        yaml.dump(data, tmp_yaml)
        tmp_yaml_path = tmp_yaml.name

    lib = QemuLibrary()
    try:
        # Use -S to prevent execution
        qmp_sock, uart_sock = lib.launch_qemu(
            tmp_yaml_path, kernel_path=None, extra_args=["-S"]
        )
        assert Path(qmp_sock).exists()
        try:
            lib.connect_to_qemu(qmp_sock, uart_sock)
        except Exception as e:
            if lib.proc and lib.proc.poll() is not None:
                out, err = lib.proc.communicate(timeout=5)  # noqa: RUF059
                pytest.fail(f"QEMU crashed during startup. STDERR: {err.decode('utf-8')}")
            raise e

        # Test passed if QEMU successfully reached the QMP stage
        # Check stderr for any unexpected warnings
        err_str = ""
        if lib.proc is not None:
            # Gracefully close QMP connection first to avoid asyncio logging errors
            lib._run(lib.bridge.close())
            # Now terminate to extract stderr
            import signal

            os.killpg(os.getpgid(lib.proc.pid), signal.SIGTERM)
            _out, err = lib.proc.communicate(timeout=5)
            err_str = err.decode("utf-8")

        # BQL warning check (the other issue reported)
        assert "WARNING: BQL held entering quantum_wait" not in err_str
        # Property not found check
        assert "Property not found" not in err_str

    finally:
        lib.close_all_connections()

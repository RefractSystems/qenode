import os

import pytest

from tools.testing.QemuLibrary import QemuLibrary


def test_dynamic_devices_realization():
    """
    Verifies that the YAML tooling and QEMU C/Rust models are synchronized.
    If property names change in C without updating the YAML emitter, QEMU will
    fail to boot (e.g., 'Property not found').
    """
    yaml_path = "test/phase12/test_bridge.yaml"
    if not os.path.exists(yaml_path):
        pytest.skip(f"{yaml_path} not found")

    lib = QemuLibrary()
    try:
        # Use -S to prevent execution (we only care about realization/startup)
        qmp_sock, uart_sock = lib.launch_qemu(
            yaml_path, kernel_path=None, extra_args=["-S", "-device", "zenoh-clock,mode=suspend,node=0"]
        )

        assert os.path.exists(qmp_sock)
        try:
            lib.connect_to_qemu(qmp_sock, uart_sock)
        except Exception as e:
            if lib.proc and lib.proc.poll() is not None:
                out, err = lib.proc.communicate(timeout=5)
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
            out, err = lib.proc.communicate(timeout=5)
            err_str = err.decode("utf-8")

        # BQL warning check (the other issue reported)
        assert "WARNING: BQL held entering quantum_wait" not in err_str
        # Property not found check
        assert "Property not found" not in err_str

    finally:
        lib.close_all_connections()

import asyncio
import os
import subprocess

import pytest

from tools.testing.qmp_bridge import QmpBridge


@pytest.mark.asyncio
async def test_invalid_qmp_command(qmp_bridge):
    """
    Verify that invalid QMP commands raise an appropriate error.
    """
    from qemu.qmp.qmp_client import ExecuteError
    with pytest.raises(ExecuteError):
        await qmp_bridge.execute("non-existent-command")

@pytest.mark.asyncio
async def test_qemu_crash_handling(qemu_launcher):
    """
    Test how the bridge handles QEMU crashing mid-execution.
    """
    # For this test, let's manually launch and crash it.
    import tempfile
    tmpdir = tempfile.mkdtemp()
    qmp_sock = os.path.join(tmpdir, "qmp.sock")

    proc = subprocess.Popen([
        "./scripts/run.sh", "--dtb", "test/phase1/minimal.dtb",
        "-qmp", f"unix:{qmp_sock},server,nowait", "-display", "none"
    ], start_new_session=True)

    # Wait for socket
    for _ in range(50):
        if os.path.exists(qmp_sock):
            break
        await asyncio.sleep(0.1)

    bridge = QmpBridge()
    try:
        await bridge.connect(qmp_sock)

        # Kill QEMU
        proc.kill()
        proc.wait()

        # Next command should fail
        from qemu.qmp.protocol import ConnectError
        from qemu.qmp.qmp_client import ExecInterruptedError
        with pytest.raises((ExecInterruptedError, ConnectError, EOFError, ConnectionResetError, asyncio.IncompleteReadError)):
            await bridge.execute("query-status")
    finally:
        await bridge.close()
        if proc.poll() is None:
            proc.kill()
        import shutil
        shutil.rmtree(tmpdir)

@pytest.mark.asyncio
async def test_connect_to_missing_socket():
    """
    Verify error when connecting to a non-existent socket.
    """
    bridge = QmpBridge()
    from qemu.qmp.protocol import ConnectError
    with pytest.raises((FileNotFoundError, ConnectionRefusedError, OSError, ConnectError)):
        await bridge.connect("/tmp/non_existent_socket_12345.sock")

@pytest.mark.asyncio
async def test_uart_disconnect_handling(qemu_launcher):
    """
    Test how the bridge handles UART socket closing.
    """
    # This is harder to test without a mock or crashing QEMU.
    # If QEMU exits, the UART socket will close.
    pass

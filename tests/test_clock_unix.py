import asyncio
import socket
import struct
import subprocess
import tempfile
from pathlib import Path

import pytest


def build_artifacts():
    workspace_root = Path(__file__).resolve().parent.parent
    dtb_path = workspace_root / "test/phase1/minimal.dtb"
    kernel_path = workspace_root / "test/phase1/hello.elf"

    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run(["make", "-C", "test/phase1", "all"], check=True)

    return dtb_path, kernel_path


class MockUnixTimeAuthority:
    def __init__(self, socket_path):
        self.socket_path = socket_path
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(self.socket_path)
        self.server.listen(1)
        self.conn = None

    async def accept(self):
        self.server.setblocking(False)
        loop = asyncio.get_running_loop()
        self.conn, _ = await loop.sock_accept(self.server)

    async def step(self, delta_ns, mujoco_time_ns):
        # ClockAdvanceReq: delta_ns (u64), mujoco_time_ns (u64)
        req = struct.pack("<QQ", delta_ns, mujoco_time_ns)
        assert self.conn is not None
        await asyncio.get_running_loop().sock_sendall(self.conn, req)

        # ClockReadyResp: current_vtime_ns (u64), n_frames (u32), error_code (u32)
        resp_data = b""
        while len(resp_data) < 16:
            assert self.conn is not None
            chunk = await asyncio.get_running_loop().sock_recv(self.conn, 16 - len(resp_data))
            if not chunk:
                raise RuntimeError("Connection closed")
            resp_data += chunk

        current_vtime, n_frames, error_code = struct.unpack("<QII", resp_data)
        return current_vtime, n_frames, error_code

    def close(self):
        if self.conn:
            self.conn.close()
        self.server.close()
        p = Path(self.socket_path)
        if p.exists():
            p.unlink()


@pytest.mark.asyncio
async def test_clock_unix_socket(qemu_launcher):
    """
    DET-4: Verify zenoh-clock with unix socket transport.
    """
    dtb_path, kernel_path = build_artifacts()

    with tempfile.TemporaryDirectory() as tmpdir:
        socket_path = str(Path(tmpdir) / "clock.sock")
        vta = MockUnixTimeAuthority(socket_path)

        extra_args = ["-S", "-device", f"zenoh-clock,node=1,mode=slaved-unix,router={socket_path}"]

        # 1. Launch QEMU. It will start, realize zenoh-clock (spawn worker),
        #    and start QMP server.
        launcher_task = asyncio.create_task(qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True))

        # 2. Wait for the worker thread to connect to our socket.
        await vta.accept()

        # 3. Handle initial sync BEFORE awaiting the bridge to unblock realize if needed
        # (Though we moved it to worker, QEMU might still be sensitive).
        # Actually, worker is calling recv_advance now.
        await vta.step(0, 0)

        # 4. Now await the bridge
        bridge = await launcher_task

        await bridge.start_emulation()

        try:
            # 2. Advance 1ms
            vtime, _, err = await vta.step(1_000_000, 1_000_000)
            assert err == 0
            assert vtime >= 1_000_000

            # 3. Advance 10ms
            vtime, _, err = await vta.step(10_000_000, 11_000_000)
            assert err == 0
            assert vtime >= 11_000_000

        finally:
            vta.close()

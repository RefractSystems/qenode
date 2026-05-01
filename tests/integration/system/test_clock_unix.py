"""
SOTA Test Module: test_clock_unix

Context:
This module implements tests for the test_clock_unix subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_clock_unix.
"""

import asyncio
import socket
import subprocess
import tempfile
from pathlib import Path

import pytest
import vproto


def build_artifacts():
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    dtb_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel_path = workspace_root / "tests/fixtures/guest_apps/boot_arm/hello.elf"

    if not dtb_path.exists() or not kernel_path.exists():
        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/boot_arm", "all"], check=True)

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
        # ClockAdvanceReq: delta_ns (u64), mujoco_time_ns (u64), quantum_number (u64)
        req = vproto.ClockAdvanceReq(delta_ns, mujoco_time_ns, 0).pack()
        assert self.conn is not None
        await asyncio.get_running_loop().sock_sendall(self.conn, req)

        # ClockReadyResp: current_vtime_ns (u64), n_frames (u32), error_code (u32), quantum_number (u64)
        resp_data = b""
        while len(resp_data) < vproto.SIZE_CLOCK_READY_RESP:
            assert self.conn is not None
            chunk = await asyncio.get_running_loop().sock_recv(self.conn, vproto.SIZE_CLOCK_READY_RESP - len(resp_data))
            if not chunk:
                raise RuntimeError("Connection closed")
            resp_data += chunk

        resp = vproto.ClockReadyResp.unpack(resp_data)
        return resp.current_vtime_ns, resp.n_frames, resp.error_code

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
    Verify clock with unix socket transport.
    """
    dtb_path, kernel_path = build_artifacts()

    with tempfile.TemporaryDirectory() as tmpdir:
        socket_path = str(Path(tmpdir) / "clock.sock")
        vta = MockUnixTimeAuthority(socket_path)

        extra_args = ["-S", "-device", f"virtmcu-clock,node=1,mode=slaved-unix,router={socket_path}"]

        # 1. Launch QEMU. It will start, realize clock (spawn worker),
        #    and start QMP server.
        launcher_task = asyncio.create_task(
            qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)
        )

        # 2. Wait for the worker thread to connect to our socket.
        await vta.accept()

        # 3. Now await the bridge
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

"""
SOTA Test Module: test_shutdown_safety

Context:
This module implements tests for the test_shutdown_safety subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_shutdown_safety.
"""

import asyncio
import contextlib
import logging
import typing
from pathlib import Path

import pytest

from tools.testing.utils import wait_for_file_creation
from tools.vproto import (
    SIZE_MMIO_REQ,
    SIZE_VIRTMCU_HANDSHAKE,
    VIRTMCU_PROTO_MAGIC,
    VIRTMCU_PROTO_VERSION,
    MmioReq,
    VirtmcuHandshake,
)

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_bridge_shutdown_safety_mmio(qemu_launcher, tmp_path):
    """
    Verify that QEMU can shut down cleanly even if a vCPU thread is blocked
    in an MMIO operation (Task C / P06).
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    mmio_sock = str(tmp_path / "mmio_shutdown.sock")
    dtb = Path(workspace_root) / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = Path(workspace_root) / "tests/fixtures/guest_apps/telemetry_wfi/test_mmio.elf"

    extra_args = [
        "-device",
        f"mmio-socket-bridge,id=bridge0,socket-path={mmio_sock},region-size=4096,base-addr=0x10000000",
    ]

    # Server that accepts connection but doesn't respond to the first MMIO request
    req_received = asyncio.Event()

    async def handle_mmio(reader, writer):
        try:
            # Handshake
            await reader.readexactly(SIZE_VIRTMCU_HANDSHAKE)
            hs_out = VirtmcuHandshake(magic=VIRTMCU_PROTO_MAGIC, version=VIRTMCU_PROTO_VERSION)
            writer.write(hs_out.pack())
            await writer.drain()

            # Read the first request but DO NOT respond
            data = await reader.readexactly(SIZE_MMIO_REQ)
            _req = MmioReq.unpack(data)
            req_received.set()

            # Keep connection open to keep vCPU blocked
            await asyncio.sleep(60)  # SLEEP_EXCEPTION: block vCPU intentionally
        except asyncio.IncompleteReadError:
            pass
        except Exception as e:
            logger.error(f"MMIO Server Error: {e}")
        finally:
            writer.close()
            with contextlib.suppress(Exception):
                await writer.wait_closed()

    server = await asyncio.start_unix_server(handle_mmio, mmio_sock)
    server_task = asyncio.create_task(server.serve_forever())

    # Wait for socket deterministically
    await wait_for_file_creation(mmio_sock)

    try:
        # Launch QEMU
        qemu = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

        # Wait for the first MMIO request to be received by our server
        from tools.testing.utils import get_time_multiplier

        try:
            await asyncio.wait_for(req_received.wait(), timeout=10.0 * get_time_multiplier())
        except TimeoutError:
            pytest.fail("Firmware did not perform expected MMIO operation")

        await qemu.execute("quit")
    finally:
        server_task.cancel()
        server.close()
        with contextlib.suppress(Exception):
            await server.wait_closed()


@pytest.mark.asyncio
async def test_bridge_shutdown_safety_remote_port(qemu_launcher, tmp_path):
    """
    Verify shutdown safety for remote-port-bridge.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    rp_sock = str(tmp_path / "rp_shutdown.sock")
    dtb = Path(workspace_root) / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    kernel = Path(workspace_root) / "tests/fixtures/guest_apps/telemetry_wfi/test_mmio.elf"

    extra_args = [
        "-device",
        f"remote-port-bridge,id=rp0,socket-path={rp_sock},region-size=4096,base-addr=0x10000000",
    ]

    import ctypes

    class RpPktHdr(ctypes.BigEndianStructure):
        _pack_ = 1
        _fields_: typing.ClassVar = [
            ("cmd", ctypes.c_uint32),
            ("len", ctypes.c_uint32),
            ("id", ctypes.c_uint32),
            ("flags", ctypes.c_uint32),
            ("dev", ctypes.c_uint32),
        ]

    class RpVersion(ctypes.BigEndianStructure):
        _pack_ = 1
        _fields_: typing.ClassVar = [
            ("major", ctypes.c_uint16),
            ("minor", ctypes.c_uint16),
        ]

    class RpCaps(ctypes.BigEndianStructure):
        _pack_ = 1
        _fields_: typing.ClassVar = [
            ("cap", ctypes.c_uint32),
            ("len", ctypes.c_uint16),
            ("res", ctypes.c_uint16),
        ]

    class RpPktBusaccess(ctypes.BigEndianStructure):
        _pack_ = 1
        _fields_: typing.ClassVar = [
            ("hdr", RpPktHdr),
            ("timestamp", ctypes.c_uint64),
            ("attrs", ctypes.c_uint64),
            ("addr", ctypes.c_uint64),
            ("len", ctypes.c_uint32),
            ("width", ctypes.c_uint32),
            ("stream_width", ctypes.c_uint32),
            ("master_id", ctypes.c_uint16),
        ]

    rp_pkt_hello_size = ctypes.sizeof(RpPktHdr) + ctypes.sizeof(RpVersion) + ctypes.sizeof(RpCaps)

    req_received = asyncio.Event()

    async def handle_rp(reader, writer):
        try:
            # Handshake: Read Hello
            await reader.readexactly(rp_pkt_hello_size)
            # Send Hello back
            writer.write(b"\x00" * rp_pkt_hello_size)  # Dummy hello
            await writer.drain()

            # Read first bus access request but DO NOT respond
            await reader.readexactly(ctypes.sizeof(RpPktBusaccess))
            req_received.set()

            # ... we just wait
            await asyncio.sleep(60)  # SLEEP_EXCEPTION: block vCPU intentionally
        except Exception as e:
            logger.error(f"RP Server Error: {e}")
        finally:
            writer.close()

    server = await asyncio.start_unix_server(handle_rp, rp_sock)
    server_task = asyncio.create_task(server.serve_forever())

    try:
        qemu = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

        from tools.testing.utils import get_time_multiplier

        try:
            await asyncio.wait_for(req_received.wait(), timeout=10.0 * get_time_multiplier())
        except TimeoutError:
            pytest.fail("Firmware did not perform expected RP operation")

        await qemu.execute("quit")
    finally:
        server_task.cancel()
        server.close()
        with contextlib.suppress(Exception):
            await server.wait_closed()

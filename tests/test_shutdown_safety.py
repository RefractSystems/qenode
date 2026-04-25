import asyncio
import contextlib
from pathlib import Path

import pytest

from tools.vproto import (
    SIZE_MMIO_REQ,
    SIZE_VIRTMCU_HANDSHAKE,
    VIRTMCU_PROTO_MAGIC,
    VIRTMCU_PROTO_VERSION,
    MmioReq,
    VirtmcuHandshake,
)


@pytest.mark.asyncio
async def test_bridge_shutdown_safety_mmio(qemu_launcher, tmp_path):
    """
    Verify that QEMU can shut down cleanly even if a vCPU thread is blocked
    in an MMIO operation (Task C / P06).
    """
    workspace_root = Path(__file__).resolve().parent.parent
    mmio_sock = str(tmp_path / "mmio_shutdown.sock")
    dtb = Path(workspace_root) / "test/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "test/phase12/test_mmio.elf"

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
            await asyncio.sleep(60)
        except asyncio.IncompleteReadError:
            pass
        except Exception as e:
            print(f"MMIO Server Error: {e}")
        finally:
            writer.close()
            with contextlib.suppress(Exception):
                await writer.wait_closed()

    server = await asyncio.start_unix_server(handle_mmio, mmio_sock)
    server_task = asyncio.create_task(server.serve_forever())

    # Wait for socket
    for _ in range(50):
        if Path(mmio_sock).exists():
            break
        await asyncio.sleep(0.1)

    try:
        # Launch QEMU
        qemu = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

        # Wait for the first MMIO request to be received by our server
        try:
            await asyncio.wait_for(req_received.wait(), timeout=10.0)
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
    workspace_root = Path(__file__).resolve().parent.parent
    rp_sock = str(tmp_path / "rp_shutdown.sock")
    dtb = Path(workspace_root) / "test/phase1/minimal.dtb"
    kernel = Path(workspace_root) / "test/phase12/test_mmio.elf"

    extra_args = [
        "-device",
        f"remote-port-bridge,id=rp0,socket-path={rp_sock},region-size=4096,base-addr=0x10000000",
    ]

    # Remote Port Hello packet
    from struct import Struct
    rp_pkt_hdr_struct = Struct(">IIIII")  # cmd, len, id, flags, dev
    rp_version_struct = Struct(">HH")
    rp_caps_struct = Struct(">IHH")
    rp_pkt_hello_size = rp_pkt_hdr_struct.size + rp_version_struct.size + rp_caps_struct.size

    req_received = asyncio.Event()

    async def handle_rp(reader, writer):
        try:
            # Handshake: Read Hello
            await reader.readexactly(rp_pkt_hello_size)
            # Send Hello back
            writer.write(b"\x00" * rp_pkt_hello_size)  # Dummy hello
            await writer.drain()

            # Read first bus access request but DO NOT respond
            # RpPktBusaccess header is RP_PKT_HDR_STRUCT + timestamp(Q) + attrs(Q) + addr(Q) + len(I) + width(I) + stream_width(I) + master_id(H)
            rp_pkt_busaccess_size = rp_pkt_hdr_struct.size + 8 + 8 + 8 + 4 + 4 + 4 + 2
            await reader.readexactly(rp_pkt_busaccess_size)
            req_received.set()

            # ... we just wait
            await asyncio.sleep(60)
        except Exception as e:
            print(f"RP Server Error: {e}")
        finally:
            writer.close()

    server = await asyncio.start_unix_server(handle_rp, rp_sock)
    server_task = asyncio.create_task(server.serve_forever())

    try:
        qemu = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

        try:
            await asyncio.wait_for(req_received.wait(), timeout=10.0)
        except TimeoutError:
            pytest.fail("Firmware did not perform expected RP operation")

        await qemu.execute("quit")
    finally:
        server_task.cancel()
        server.close()
        with contextlib.suppress(Exception):
            await server.wait_closed()

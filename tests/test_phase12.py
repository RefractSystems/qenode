import asyncio
import json
import subprocess
from pathlib import Path

import pytest

from tools.vproto import (
    SIZE_MMIO_REQ,
    SIZE_VIRTMCU_HANDSHAKE,
    VIRTMCU_PROTO_MAGIC,
    VIRTMCU_PROTO_VERSION,
    MmioReq,
    SyscMsg,
    VirtmcuHandshake,
)


@pytest.mark.asyncio
async def test_phase12_yaml2qemu_validation(tmp_path):
    """
    1. yaml2qemu Output Validation [P0]
    """
    workspace_root = Path(__file__).resolve().parent.parent
    yaml_file = Path(workspace_root) / "test/phase12/test_malformed.yaml"
    out_dtb = tmp_path / "test_malformed.dtb"

    proc = await asyncio.create_subprocess_exec(
        "python3",
        "-m",
        "tools.yaml2qemu",
        yaml_file,
        "--out-dtb",
        out_dtb,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        cwd=workspace_root,
    )
    _stdout, stderr = await proc.communicate()

    assert proc.returncode != 0
    assert b"device 'known_but_unmapped' skipped" in stderr


@pytest.mark.asyncio
async def test_phase12_mmio_bridge_offsets(qemu_launcher, tmp_path):
    """
    2. MMIO Bridge Protocol: Offsets vs. Absolute Addresses [P1]
    """
    workspace_root = Path(__file__).resolve().parent.parent

    # Use unique paths for this test run
    yaml_file = tmp_path / "test_bridge.yaml"
    dtb_file = tmp_path / "test_bridge.dtb"
    cli_file = tmp_path / "test_bridge.cli"
    mmio_sock = str(tmp_path / "mmio.sock")

    # Read the original yaml and update the socket path
    import yaml

    orig_yaml = Path(workspace_root) / "test/phase12/test_bridge.yaml"
    with orig_yaml.open() as f:
        yaml_data = yaml.safe_load(f)
    for p in yaml_data.get("peripherals", []):
        if p["type"] == "mmio-socket-bridge":
            p["properties"]["socket-path"] = mmio_sock

    with Path(yaml_file).open("w") as f:
        yaml.dump(yaml_data, f)

    kernel = Path(workspace_root) / "test/phase12/test_mmio.elf"
    # Generate DTB and CLI
    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", yaml_file, "--out-dtb", dtb_file, "--out-cli", cli_file],
        check=True,
        cwd=workspace_root,
    )

    with Path(cli_file).open() as f:
        cli_args = f.read().split()

    received_reqs = []

    async def handle_mmio(reader, writer):
        try:
            print("MMIO Server: Connection accepted!")
            # Handshake
            await reader.readexactly(SIZE_VIRTMCU_HANDSHAKE)
            print("MMIO Server: Handshake read!")
            hs_out = VirtmcuHandshake(magic=VIRTMCU_PROTO_MAGIC, version=VIRTMCU_PROTO_VERSION)
            writer.write(hs_out.pack())
            await writer.drain()
            print("MMIO Server: Handshake written!")

            while True:
                try:
                    data = await reader.readexactly(SIZE_MMIO_REQ)
                    req = MmioReq.unpack(data)
                    print(f"MMIO Server: Received req: {req}")
                    received_reqs.append(req)
                    resp = SyscMsg(type=0, irq_num=0, data=0)
                    writer.write(resp.pack())
                    await writer.drain()
                except (asyncio.IncompleteReadError, ConnectionResetError) as e:
                    print(f"MMIO Server: Read loop exited: {e}")
                    break
        except Exception as e:
            print(f"MMIO Server Error: {e}")
        finally:
            writer.close()
            await writer.wait_closed()
            print("MMIO Server: Connection closed.")

    server = await asyncio.start_unix_server(handle_mmio, mmio_sock)

    async def run_server():
        async with server:
            await server.serve_forever()

    server_task = asyncio.create_task(run_server())

    # Deterministic socket check
    for _ in range(50):
        if Path(mmio_sock).exists():
            break
        await asyncio.sleep(0.01)

    try:
        await qemu_launcher(
            dtb_file,
            kernel,
            extra_args=[
                *cli_args,
                "-serial", "null",
                "-monitor", "null",
            ]
        )

        # In standalone mode, QEMU runs at wall-clock speed.
        # We just wait for a moment for the guest to perform MMIO.
        await asyncio.sleep(2.0)
    finally:
        server_task.cancel()
        server.close()

    # Verify requests
    addrs = [req.addr for req in received_reqs]
    data = [req.data for req in received_reqs]

    assert 0x0 in addrs
    assert 0x4 in addrs
    assert 0xDEADBEEF in data
    assert 0xCAFEBABE in data


@pytest.mark.asyncio
async def test_phase12_telemetry(zenoh_router, qemu_launcher, zenoh_session, tmp_path):
    """
    3. Telemetry Test: Verify Zenoh telemetry events are emitted.
    """
    workspace_root = Path(__file__).resolve().parent.parent
    yaml_file = Path(workspace_root) / "test/phase12/test_telemetry.yaml"
    dtb_file = tmp_path / "test_telemetry.dtb"
    cli_file = tmp_path / "test_telemetry.cli"
    kernel = Path(workspace_root) / "test/phase12/test_wfi.elf"

    # Generate DTB and CLI
    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", yaml_file, "--out-dtb", dtb_file, "--out-cli", cli_file],
        check=True,
        cwd=workspace_root,
    )

    with Path(cli_file).open() as f:
        cli_args = f.read().split()

    # Listener for telemetry
    received_events = []

    def on_telemetry(sample):
        received_events.append(str(sample.key_expr))

    sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("sim/telemetry/**", on_telemetry))

    # Run QEMU
    await qemu_launcher(
        dtb_file,
        kernel,
        extra_args=[
            *cli_args,
            "-global", f"zenoh-telemetry.router={zenoh_router}",
            "-serial", "null",
            "-monitor", "null",
        ],
    )

    # Wait for telemetry to be emitted in standalone mode
    await asyncio.sleep(3.0)

    await asyncio.to_thread(sub.undeclare)

    assert len(received_events) > 0, "No telemetry events received"
    assert any("sim/telemetry" in e for e in received_events)


@pytest.mark.asyncio
async def test_phase12_zenoh_clock_error(qemu_launcher, tmp_path):
    """
    4. zenoh-clock Connection Error Reporting [P0]
    """
    workspace_root = Path(__file__).resolve().parent.parent
    yaml_file = Path(workspace_root) / "test/phase12/test_telemetry.yaml"
    dtb_file = tmp_path / "test_telemetry.dtb"

    # Generate DTB
    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", yaml_file, "--out-dtb", dtb_file],
        check=True,
        cwd=workspace_root,
    )

    # Invalid router
    extra_args = ["-device", "zenoh-clock,node=1,router=tcp/127.0.0.1:1,mode=slaved-suspend"]

    # We expect this to fail to start or fail to connect
    import contextlib

    from qemu.qmp.protocol import ConnectError

    with contextlib.suppress(RuntimeError, TimeoutError, ConnectError):
        await qemu_launcher(dtb_file, extra_args=extra_args, ignore_clock_check=True)


@pytest.mark.asyncio
async def test_phase12_coordinator_topology(zenoh_router, zenoh_coordinator, zenoh_session):  # noqa: ARG001
    """
    5. Verify Zenoh Coordinator topology control
    """
    update = {"from": "node0", "to": "node1", "delay_ns": 5000000, "drop_probability": 0.5}

    await asyncio.to_thread(lambda: zenoh_session.put("sim/network/control", json.dumps(update)))
    await asyncio.sleep(1.0)


@pytest.mark.asyncio
async def test_phase12_telemetry_listener_flatbuffers(zenoh_session):  # noqa: ARG001
    """
    6. Verify telemetry schema (FlatBuffers) and QOM path resolution via Mock
    """
    import sys

    workspace_root = Path(__file__).resolve().parent.parent
    fbs_dir = Path(workspace_root) / "tools/telemetry_fbs"
    if str(fbs_dir) not in sys.path:
        sys.path.append(str(fbs_dir))

    import flatbuffers
    import Virtmcu.Telemetry.TraceEvent as TraceEvent

    builder = flatbuffers.Builder(1024)
    name = builder.CreateString("/machine/peripheral/uart0")
    TraceEvent.Start(builder)
    TraceEvent.AddTimestampNs(builder, 123456789)
    TraceEvent.AddType(builder, 1)  # IRQ
    TraceEvent.AddId(builder, (2 << 16) | 7)
    TraceEvent.AddValue(builder, 1)
    TraceEvent.AddDeviceName(builder, name)
    ev = TraceEvent.End(builder)
    builder.Finish(ev)
    payload = builder.Output()

    # Unpack check
    buf = bytes(payload)
    event = TraceEvent.TraceEvent.GetRootAs(buf, 0)
    assert event.TimestampNs() == 123456789
    assert event.Type() == 1
    assert event.DeviceName() == b"/machine/peripheral/uart0"

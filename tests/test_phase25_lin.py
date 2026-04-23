import asyncio
import subprocess
import sys
from pathlib import Path

import flatbuffers
import pytest

# Add tools/lin_fbs to sys.path
sys.path.append(str(Path.cwd() / "tools/lin_fbs"))

from virtmcu.lin import LinFrame, LinMessageType


def create_lin_frame(vtime_ns, msg_type, data):
    builder = flatbuffers.Builder(1024)
    data_offset = None
    if data:
        data_offset = builder.CreateByteVector(data)

    LinFrame.Start(builder)
    LinFrame.AddDeliveryVtimeNs(builder, vtime_ns)
    LinFrame.AddType(builder, msg_type)
    if data_offset is not None:
        LinFrame.AddData(builder, data_offset)
    frame = LinFrame.End(builder)
    builder.Finish(frame)
    return builder.Output()


@pytest.mark.asyncio
async def test_lin_lpuart(zenoh_router, qemu_launcher, zenoh_session):
    import shutil
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-lin-")

    router_endpoint = zenoh_router

    # 1. Build ELF
    kernel = Path(tmpdir) / "lin_echo.elf"
    subprocess.run(
        f"arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T test/phase25/lin_echo.ld test/phase25/lin_echo.S -o {kernel}",
        shell=True,
        check=True,
    )

    # Use unique topic to avoid interference
    import uuid

    unique_id = str(uuid.uuid4())[:8]
    lin_topic = f"sim/lin/{unique_id}"

    # Generate DTB
    dtb = Path(tmpdir) / "lin_test.dtb"
    subprocess.run(
        f"sed -e 's|tcp/127.0.0.1:7447|{router_endpoint}|' -e 's|\"sim/lin\"|\"{lin_topic}\"|' test/phase25/lin_test.dts | dtc -I dts -O dtb -o {dtb}",
        shell=True,
        check=True,
    )
    extra_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n0",
        "-serial",
        "chardev:n0",
        "-icount",
        "shift=0,align=off,sleep=off",
        "-net",
        "none",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=0,router={router_endpoint}",
        "-device",
        f"s32k144-lpuart,node=0,router={router_endpoint},topic={lin_topic}",
    ]

    print(f"Starting QEMU with topic {lin_topic}...")
    await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

    # 2. Connect to Zenoh
    session = zenoh_session

    received = []

    def on_msg(sample):
        try:
            payload = sample.payload.to_bytes()
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])
            print(f"Received from QEMU: type={msg_type}, data={data!r}")
            received.append((msg_type, data))
        except Exception as e:
            print(f"Callback error: {e}")

    tx_topic = f"{lin_topic}/0/tx"
    rx_topic = f"{lin_topic}/0/rx"

    # We use VirtualTimeAuthority from conftest for cleaner clock driving
    from tests.conftest import VirtualTimeAuthority

    vta = VirtualTimeAuthority(session, [0])

    sub = await asyncio.to_thread(lambda: session.declare_subscriber(tx_topic, on_msg))
    pub = await asyncio.to_thread(lambda: session.declare_publisher(rx_topic))

    try:
        # Initial clock sync
        await vta.step(0)

        print("Sending 'X' to QEMU RX...")
        frame = create_lin_frame(1_000_000, LinMessageType.LinMessageType.Data, b"X")
        await asyncio.to_thread(lambda: pub.put(frame))

        # Advance clock to process 'X'
        await vta.step(5_000_000)

        print("Sending Break to QEMU RX...")
        frame = create_lin_frame(6_000_000, LinMessageType.LinMessageType.Break, None)
        await asyncio.to_thread(lambda: pub.put(frame))

        # Advance clock to process Break
        await vta.step(5_000_000)

        # Deterministic check for responses
        print("Checking responses...")
        found_x = False
        found_b = False
        for _ in range(10):
            for msg_type, data in received:
                if msg_type == LinMessageType.LinMessageType.Data:
                    if data == b"X":
                        found_x = True
                    if data == b"B":
                        found_b = True
            if found_x and found_b:
                break
            await vta.step(5_000_000)

        assert found_x, f"Failed to receive Echo for 'X', received: {received}"
        assert found_b, f"Failed to receive Echo for Break, received: {received}"

        print("SUCCESS: Phase 25 LIN UART verified.")

    finally:
        await asyncio.to_thread(sub.undeclare)
        shutil.rmtree(tmpdir)

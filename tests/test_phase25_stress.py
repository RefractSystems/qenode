import asyncio
import subprocess
import sys
from pathlib import Path

import pytest

# Add tools/lin_fbs to sys.path
sys.path.append(str(Path.cwd() / "tools/lin_fbs"))

import flatbuffers
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
async def test_lin_stress(zenoh_router, qemu_launcher, zenoh_session):
    import shutil
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-lin-stress-")

    router_endpoint = zenoh_router

    # Build ELF
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
        f"zenoh-clock,mode=slaved-icount,node=0,router={router_endpoint},stall-timeout=30000",
        # The s32k144-lpuart device is instantiated by the DTB, no need for -device
    ]

    print(f"Starting QEMU with topic {lin_topic}...")
    await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

    # 2. Connect to Zenoh
    session = zenoh_session

    received_count = 0
    errors = 0

    def on_bus_msg(sample):
        nonlocal received_count, errors
        try:
            payload = sample.payload.to_bytes()
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            # Count any data frame received on node 0 TX topic
            if frame.Type() == LinMessageType.LinMessageType.Data:
                received_count += 1
        except Exception:
            errors += 1

    # Listen to Node 0 TX and publish to Node 0 RX
    # Note: s32k144-lpuart uses {topic}/{node_id}/tx and {topic}/{node_id}/rx
    tx_topic = f"{lin_topic}/0/tx"
    rx_topic = f"{lin_topic}/0/rx"
    sub = await asyncio.to_thread(lambda: session.declare_subscriber(tx_topic, on_bus_msg))
    pub = await asyncio.to_thread(lambda: session.declare_publisher(rx_topic))

    from conftest import TimeAuthority

    ta = TimeAuthority(session, node_id=0)

    try:
        # Initial clock sync
        await ta.step(0)

        print("Starting staggered frame injection...")
        step_ns = 1_000_000  # 1ms steps
        total_steps = 100

        for i in range(total_steps):
            # Send one frame every ms
            frame = create_lin_frame(i * step_ns, LinMessageType.LinMessageType.Data, b"S")
            from functools import partial

            await asyncio.to_thread(partial(pub.put, frame))

            # Advance clock by 1ms
            await ta.step(step_ns)

        print(f"Received {received_count} echo responses, {errors} errors.")
        assert received_count > 0, "No responses received!"
        print(f"SUCCESS: Received {received_count} responses.")

    finally:
        await asyncio.to_thread(sub.undeclare)
        shutil.rmtree(tmpdir)

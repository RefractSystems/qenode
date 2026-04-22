import asyncio
import os
from pathlib import Path

import pytest
import zenoh
from conftest import TimeAuthority

WORKSPACE_DIR = Path(__file__).resolve().parent.parent


def build_flexray_artifacts(router_endpoint="tcp/127.0.0.1:7447"):
    phase27_dir = Path(WORKSPACE_DIR) / "test/phase27"
    import subprocess

    firmware_s = """
.global _start
_start:
    /* 1. Configure Message RAM via Interface */

    /* Slot 0: Frame ID = 10, for TX test */
    ldr r0, =0x09003400
    mov r1, #10
    str r1, [r0]
    ldr r0, =0x09003404
    ldr r1, =0x000800FF
    str r1, [r0]
    ldr r0, =0x09003410
    ldr r1, =0xDEC0ADDE
    str r1, [r0]
    ldr r0, =0x09003500
    mov r1, #0
    str r1, [r0]

    /* Slot 1: Frame ID = 20, for RX test */
    ldr r0, =0x09003400
    mov r1, #20
    str r1, [r0]
    ldr r0, =0x09003404
    ldr r1, =0x000800FF
    str r1, [r0]
    ldr r0, =0x09003500
    mov r1, #1
    str r1, [r0]

    /* 2. Start controller */
    ldr r0, =0x09003080
    mov r1, #4
    str r1, [r0]

wait_loop:
    /* 3. Check Message RAM Slot 1 for data */
    /* We'll use OBCR to read it back into ORDS */
    ldr r0, =0x09003700
    mov r1, #1
    str r1, [r0]

    /* Check ORDS1 for our expected RX value 0xCAFEBABE */
    ldr r0, =0x09003610
    ldr r1, [r0]
    ldr r2, =0xCAFEBABE
    cmp r1, r2
    bne wait_loop

    /* 4. Signal success via WRHS3 */
    ldr r0, =0x09003408
    ldr r1, =0x12345678
    str r1, [r0]

loop:
    nop
    b loop
"""
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-flexray-")
    # Copy all files from phase27 to tmpdir
    import shutil

    for f in Path(phase27_dir).iterdir():
        if f.is_file():
            shutil.copy(f, tmpdir)

    with (Path(tmpdir) / "firmware.S").open("w") as fw_file:
        fw_file.write(firmware_s)

    subprocess.run(["make", "-C", tmpdir, "clean", "all"], check=True)
    dtb_path = str(Path(tmpdir) / "platform.dtb")
    yaml_path = Path(tmpdir) / "platform.yaml"

    # Update router in YAML
    with yaml_path.open("r") as yaml_file:
        content = yaml_file.read()
    content = content.replace("tcp/127.0.0.1:7447", router_endpoint)
    with yaml_path.open("w") as yaml_file:
        yaml_file.write(content)

    # Always regenerate DTB for safety
    env = os.environ.copy()
    env["PYTHONPATH"] = str(WORKSPACE_DIR)
    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", str(yaml_path), "--out-dtb", dtb_path],
        check=True,
        env=env,
    )
    return tmpdir


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_flexray_zenoh_tx(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 27: Verify FlexRay data transmission over Zenoh.
    """
    phase27_dir = build_flexray_artifacts(router_endpoint=zenoh_router)
    dtb_path = Path(phase27_dir) / "platform.dtb"
    kernel_path = Path(phase27_dir) / "firmware.elf"

    # Use unique topic to avoid interference between parallel workers
    import uuid

    unique_id = str(uuid.uuid4())[:8]
    topic = f"sim/flexray/{unique_id}"

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=0,router={zenoh_router},stall-timeout=30000",
        "-global",
        f"zenoh-flexray.topic={topic}",
    ]

    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    ta = TimeAuthority(zenoh_session, node_id=0)

    # Subscribe to FlexRay TX topic
    tx_topic = f"{topic}/0/tx"
    queue: asyncio.Queue[zenoh.Sample] = asyncio.Queue()
    loop = asyncio.get_running_loop()

    def on_msg(sample):
        loop.call_soon_threadsafe(queue.put_nowait, sample)

    sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(tx_topic, on_msg))

    # Wait for discovery (crucial for parallel runs)
    await asyncio.sleep(3.0)

    # Initial sync (Wait for discovery)
    await asyncio.sleep(2.0)
    await ta.step(0)

    # Advance time to trigger first cycle (5ms)
    target_vtime = 15_000_000
    while ta.current_vtime_ns < target_vtime:
        delta = target_vtime - ta.current_vtime_ns
        await ta.step(delta)

    # Check if we received the message
    try:
        sample = await asyncio.wait_for(queue.get(), timeout=15.0)

        import sys

        sys.path.append(str(Path(__file__).resolve().parent.parent / "tools/flexray_fbs"))
        from virtmcu.flexray.FlexRayFrame import FlexRayFrame

        buf = sample.payload.to_bytes()
        frame = FlexRayFrame.GetRootAs(buf, 0)

        data = bytes(frame.Data(i) for i in range(frame.DataLength()))
        print(f"[flexray] Received Zenoh frame: ID={frame.FrameId()}, Data={data.hex()}")

        assert frame.FrameId() == 10
        assert b"\xde\xad\xc0\xde" in data

    except TimeoutError:
        pytest.fail("Timed out waiting for FlexRay TX message on Zenoh")
    finally:
        await asyncio.to_thread(sub.undeclare)


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_flexray_zenoh_rx(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 27: Verify FlexRay data reception over Zenoh.
    """
    phase27_dir = build_flexray_artifacts(router_endpoint=zenoh_router)
    dtb_path = Path(phase27_dir) / "platform.dtb"
    kernel_path = Path(phase27_dir) / "firmware.elf"

    # Use unique topic to avoid interference between parallel workers
    import uuid

    unique_id = str(uuid.uuid4())[:8]
    topic = f"sim/flexray/{unique_id}"

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=0,router={zenoh_router},stall-timeout=30000",
        "-global",
        f"zenoh-flexray.topic={topic}",
    ]

    bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)
    ta = TimeAuthority(zenoh_session, node_id=0)

    # Initial sync (Wait for discovery)
    await asyncio.sleep(2.0)
    await ta.step(0)

    # Prepare Zenoh frame to send
    import sys

    import flatbuffers

    sys.path.append(str(Path(__file__).resolve().parent.parent / "tools/flexray_fbs"))
    from virtmcu.flexray import FlexRayFrame

    builder = flatbuffers.Builder(1024)
    data_bytes = b"\xbe\xba\xfe\xca" + b"\x00" * 12  # 0xCAFEBABE

    FlexRayFrame.StartDataVector(builder, len(data_bytes))
    for b in reversed(data_bytes):
        builder.PrependByte(b)
    data_offset = builder.EndVector()

    FlexRayFrame.Start(builder)
    FlexRayFrame.AddFrameId(builder, 20)
    FlexRayFrame.AddData(builder, data_offset)
    FlexRayFrame.AddDeliveryVtimeNs(builder, 1_000_000)
    FlexRayFrame.AddCycleCount(builder, 0)
    FlexRayFrame.AddChannel(builder, 0)
    FlexRayFrame.AddFlags(builder, 0)
    frame_offset = FlexRayFrame.End(builder)
    builder.Finish(frame_offset)

    payload = builder.Output()

    # Put message on the Zenoh topic
    await asyncio.to_thread(lambda: zenoh_session.put(topic, payload))
    # Wait for discovery/delivery (crucial for parallel runs)
    await asyncio.sleep(3.0)

    # Advance time and poll for success signal
    qom_path = "/flexray"
    success = False
    target_vtime = 50_000_000
    while ta.current_vtime_ns < target_vtime:
        await ta.step(1_000_000)
        # Check wrhs3 property for success signal
        wrhs3 = await bridge.qmp.execute("qom-get", {"path": qom_path, "property": "wrhs3"})
        if wrhs3 == 0x12345678:
            print(f"[flexray] Success signal detected at {ta.current_vtime_ns}ns: 0x{wrhs3:08x}")
            success = True
            break

    assert success, f"Failed to detect success signal. Last WRHS3: 0x{wrhs3:08x}, vtime: {ta.current_vtime_ns}"


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_flexray_stress(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 27: Verify FlexRay controller under heavy load to prevent deadlocks
    or BQL starvation.
    """
    phase27_dir = build_flexray_artifacts(router_endpoint=zenoh_router)
    dtb_path = Path(phase27_dir) / "platform.dtb"
    kernel_path = Path(phase27_dir) / "firmware.elf"

    # Use unique topic to avoid interference between parallel workers
    import uuid

    unique_id = str(uuid.uuid4())[:8]
    topic = f"sim/flexray/{unique_id}"

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=0,router={zenoh_router},stall-timeout=30000",
        "-global",
        f"zenoh-flexray.topic={topic}",
    ]

    bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)
    ta = TimeAuthority(zenoh_session, node_id=0)

    # Initial sync (Wait for discovery)
    await asyncio.sleep(2.0)
    await ta.step(0)

    import sys

    import flatbuffers

    sys.path.append(str(Path(__file__).resolve().parent.parent / "tools/flexray_fbs"))
    from virtmcu.flexray import FlexRayFrame

    # Send 100 messages into Zenoh to flood the FlexRay receiver
    for i in range(100):
        builder = flatbuffers.Builder(128)
        data_bytes = b"\x00" * 4
        FlexRayFrame.StartDataVector(builder, len(data_bytes))
        for b in reversed(data_bytes):
            builder.PrependByte(b)
        data_offset = builder.EndVector()
        FlexRayFrame.Start(builder)
        FlexRayFrame.AddFrameId(builder, 10 + (i % 10))
        FlexRayFrame.AddData(builder, data_offset)
        # Deliver them staggered across virtual time
        FlexRayFrame.AddDeliveryVtimeNs(builder, 1_000_000 + (i * 10_000))
        FlexRayFrame.AddCycleCount(builder, 0)
        FlexRayFrame.AddChannel(builder, 0)
        FlexRayFrame.AddFlags(builder, 0)
        frame_offset = FlexRayFrame.End(builder)
        builder.Finish(frame_offset)
        payload = builder.Output()
        from functools import partial

        await asyncio.to_thread(partial(zenoh_session.put, topic, payload))

    # Advance time sequentially to process all packets
    target_vtime = 15_000_000
    while ta.current_vtime_ns < target_vtime:
        await ta.step(1_000_000)

    # If QEMU didn't crash or deadlock, the test passes
    qom_path = "/flexray"
    wrhs3 = await bridge.qmp.execute("qom-get", {"path": qom_path, "property": "wrhs3"})
    assert isinstance(wrhs3, int)

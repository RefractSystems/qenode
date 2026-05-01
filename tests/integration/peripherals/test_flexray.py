"""
SOTA Test Module: test_flexray

Context:
This module implements tests for the test_flexray subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_flexray.
"""

import asyncio
import logging
import subprocess
import uuid
from functools import partial
from pathlib import Path
from typing import Any

import pytest

logger = logging.getLogger(__name__)

def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent

WORKSPACE_DIR = _find_workspace_root(Path(__file__).resolve())


def build_flexray_artifacts():
    from tools.testing.env import WORKSPACE_ROOT
    workspace_dir = WORKSPACE_ROOT
    flexray_dir = Path(workspace_dir) / "tests/fixtures/guest_apps/flexray_bridge"
    flexray_dir.mkdir(parents=True, exist_ok=True)

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

    /* Slot 2: Frame ID = 30, for loopback */
    ldr r0, =0x09003400
    mov r1, #30
    str r1, [r0]
    ldr r0, =0x09003404
    ldr r1, =0x000800FF
    str r1, [r0]
    ldr r0, =0x09003500
    mov r1, #2
    str r1, [r0]

    /* 2. Global Config */
    ldr r0, =0x09003000
    ldr r1, =0x00000001
    str r1, [r0]

loop:
    /* Check Slot 1 (RX) Status */
    ldr r0, =0x09003408
    ldr r1, [r0]
    tst r1, #1
    beq skip_rx

    /* Read Slot 1 (RX) Data */
    ldr r0, =0x09003410
    ldr r1, [r0]

    /* Write to UART 0x09000000 */
    ldr r2, =0x09000000
    str r1, [r2]

    /* Clear Status */
    ldr r0, =0x09003408
    mov r1, #0
    str r1, [r0]

skip_rx:
    b loop
"""
    with (flexray_dir / "firmware.S").open("w") as f:
        f.write(firmware_s)

    with (flexray_dir / "linker.ld").open("w") as f:
        f.write("""
ENTRY(_start)
SECTIONS {
    . = 0x40000000;
    .text : { *(.text) }
    .data : { *(.data) }
}
""")

    subprocess.run(["arm-none-eabi-as", "-o", "firmware.o", "firmware.S"], cwd=flexray_dir, check=True)
    subprocess.run(
        ["arm-none-eabi-ld", "-T", "linker.ld", "-o", "firmware.elf", "firmware.o"], cwd=flexray_dir, check=True
    )

    dts = """
/dts-v1/;
/ {
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;
    qemu_sysmem {
        container = <1>;
    };
    memory@40000000 {
        device_type = "memory";
        reg = <0x0 0x40000000 0x0 0x10000000>;
    };
    cpus {
        #address-cells = <1>;
        #size-cells = <0>;
        cpu@0 {
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
        };
    };
    flexray@09003000 {
        compatible = "virtmcu,flexray";
        reg = <0x0 0x09003000 0x0 0x1000>;
    };
    uart@09000000 {
        compatible = "pl011";
        reg = <0x0 0x09000000 0x0 0x1000>;
    };
};
"""
    with (flexray_dir / "platform.dts").open("w") as f:
        f.write(dts)
    subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", "platform.dtb", "platform.dts"], cwd=flexray_dir, check=True)
    return flexray_dir


@pytest.mark.asyncio
async def test_flexray_zenoh_tx(simulation, zenoh_router):
    """
    Verify FlexRay data transmission over Zenoh.
    """
    flexray_dir = build_flexray_artifacts()
    dtb_path = Path(flexray_dir) / "platform.dtb"
    kernel_path = Path(flexray_dir) / "firmware.elf"

    unique_id = str(uuid.uuid4())[:8]
    topic = f"sim/flexray/{unique_id}"

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        "virtmcu-clock,mode=slaved-icount,node=0",
        "-global",
        f"flexray.topic={topic}",
        "-global",
        f"flexray.router={zenoh_router}",
    ]

    tx_topic = f"{topic}/0/tx"
    queue: asyncio.Queue[Any] = asyncio.Queue()
    loop = asyncio.get_running_loop()

    def on_msg(sample):
        loop.call_soon_threadsafe(queue.put_nowait, sample)

    async with await simulation(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True) as sim:
        await asyncio.to_thread(lambda: sim.vta.session.declare_subscriber(tx_topic, on_msg))

        # Run for 20ms virtual time
        for _ in range(20):
            await sim.vta.step(1_000_000)
            if not queue.empty():
                break

        assert not queue.empty(), "No FlexRay frames received over Zenoh"
        sample = queue.get_nowait()
        assert b"\\xDE\\xAD\\xC0\\xDE" in sample.payload.to_bytes()


@pytest.mark.asyncio
async def test_flexray_zenoh_rx(simulation, zenoh_router):
    """
    Verify FlexRay data reception from Zenoh.
    """
    flexray_dir = build_flexray_artifacts()
    dtb_path = Path(flexray_dir) / "platform.dtb"
    kernel_path = Path(flexray_dir) / "firmware.elf"

    unique_id = str(uuid.uuid4())[:8]
    topic = f"sim/flexray/{unique_id}"
    rx_topic = f"{topic}/0/rx"

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        "virtmcu-clock,mode=slaved-icount,node=0",
        "-global",
        f"flexray.topic={topic}",
        "-global",
        f"flexray.router={zenoh_router}",
    ]

    import sys

    import flatbuffers

    sys.path.append(str(Path(WORKSPACE_DIR) / "tools/flexray_fbs"))
    from virtmcu.flexray import FlexRayFrame

    async with await simulation(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True) as sim:
        pub = await asyncio.to_thread(lambda: sim.vta.session.declare_publisher(rx_topic))

        builder = flatbuffers.Builder(1024)
        data_off = builder.CreateByteVector(b"\\xEF\\xBE\\xAD\\xDE")
        FlexRayFrame.Start(builder)
        FlexRayFrame.AddFrameId(builder, 20)
        FlexRayFrame.AddPayload(builder, data_off)
        FlexRayFrame.AddDeliveryVtimeNs(builder, 5_000_000)
        frame_off = FlexRayFrame.End(builder)
        builder.Finish(frame_off)

        await asyncio.to_thread(partial(pub.put, builder.Output()))

        for _ in range(20):
            await sim.vta.step(1_000_000)
            uart_data = await sim.bridge.read_uart()
            if b"\\xDE\\xAD\\xBE\\xEF" in uart_data:
                break
        else:
            uart_data = await sim.bridge.read_uart()
            assert b"\\xDE\\xAD\\xBE\\xEF" in uart_data


@pytest.mark.asyncio
async def test_flexray_stress(simulation, zenoh_router):
    """
    Verify FlexRay controller under heavy load.
    """
    flexray_dir = build_flexray_artifacts()
    dtb_path = Path(flexray_dir) / "platform.dtb"
    kernel_path = Path(flexray_dir) / "firmware.elf"

    unique_id = str(uuid.uuid4())[:8]
    topic = f"sim/flexray/{unique_id}"
    rx_topic = f"{topic}/0/rx"

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        "virtmcu-clock,mode=slaved-icount,node=0",
        "-global",
        f"flexray.topic={topic}",
        "-global",
        f"flexray.router={zenoh_router}",
    ]

    import sys

    import flatbuffers

    sys.path.append(str(Path(WORKSPACE_DIR) / "tools/flexray_fbs"))
    from virtmcu.flexray import FlexRayFrame

    async with await simulation(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True) as sim:
        pub = await asyncio.to_thread(lambda: sim.vta.session.declare_publisher(rx_topic))

        for i in range(100):
            builder = flatbuffers.Builder(64)
            data_off = builder.CreateByteVector(b"STRESS")
            FlexRayFrame.Start(builder)
            FlexRayFrame.AddFrameId(builder, 20)
            FlexRayFrame.AddPayload(builder, data_off)
            FlexRayFrame.AddDeliveryVtimeNs(builder, 1_000_000 + (i * 10_000))
            frame_off = FlexRayFrame.End(builder)
            builder.Finish(frame_off)
            await asyncio.to_thread(partial(pub.put, builder.Output()))

        await sim.vta.run_for(50_000_000)

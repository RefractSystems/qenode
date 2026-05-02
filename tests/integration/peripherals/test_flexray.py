"""
SOTA Test Module: test_flexray

Context:
This module implements tests for the test_flexray subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_flexray.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
import shutil
import subprocess
from functools import partial
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest
import zenoh

from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from typing import Any

    from tools.testing.virtmcu_test_suite.simulation import Simulation


from tools.testing.env import WORKSPACE_DIR

logger = logging.getLogger(__name__)


def build_flexray_artifacts() -> Path:
    workspace_dir = WORKSPACE_DIR
    flexray_dir = Path(workspace_dir) / "tests/fixtures/guest_apps/flexray_bridge"
    flexray_dir.mkdir(parents=True, exist_ok=True)

    firmware_s = """
.global _start
_start:
    /* Print BOOT to UART */
    ldr r2, =0x09000000
    mov r3, #66  /* 'B' */
    str r3, [r2]
    mov r3, #79  /* 'O' */
    str r3, [r2]
    str r3, [r2]
    mov r3, #84  /* 'T' */
    str r3, [r2]
    mov r3, #10 /* '\\n' */
    str r3, [r2]

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

    /* Write to UART 0x09000000 - 4 bytes individually */
    ldr r2, =0x09000000
    /* Byte 0 */
    strb r1, [r2]
    /* Byte 1 */
    lsr r3, r1, #8
    strb r3, [r2]
    /* Byte 2 */
    lsr r3, r1, #16
    strb r3, [r2]
    /* Byte 3 */
    lsr r3, r1, #24
    strb r3, [r2]

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

    subprocess.run(
        [shutil.which("arm-none-eabi-as") or "arm-none-eabi-as", "-o", "firmware.o", "firmware.S"],
        cwd=flexray_dir,
        check=True,
    )
    subprocess.run(
        [shutil.which("arm-none-eabi-ld") or "arm-none-eabi-ld", "-T", "linker.ld", "-o", "firmware.elf", "firmware.o"],
        cwd=flexray_dir,
        check=True,
    )

    dts = """
/dts-v1/;
/ {
    compatible = "arm,generic-fdt";
    #address-cells = <2>;
    #size-cells = <2>;
    qemu_sysmem: qemu_sysmem {
        compatible = "qemu:system-memory";
        phandle = <0x01>;
    };
    memory@40000000 {
        compatible = "qemu-memory-region";
        qemu,ram = <1>;
        container = <1>;
        reg = <0x0 0x40000000 0x0 0x08000000>;
    };
    cpus {
        #address-cells = <1>;
        #size-cells = <0>;
        cpu@0 {
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
            memory = <0x01>;
        };
    };
    flexray@9003000 {
        compatible = "flexray";
        reg = <0x0 0x09003000 0x0 0x4000>;
        container = <1>;
    };
    pl011@9000000 {
        compatible = "pl011";
        reg = <0x0 0x09000000 0x0 0x1000>;
        chardev = <0x00>;
        container = <1>;
    };
};
"""
    with (flexray_dir / "platform.dts").open("w") as f:
        f.write(dts)
    subprocess.run(
        [shutil.which("dtc") or "dtc", "-I", "dts", "-O", "dtb", "-o", "platform.dtb", "platform.dts"],
        cwd=flexray_dir,
        check=True,
    )
    return flexray_dir


@pytest.mark.asyncio
async def test_flexray_zenoh_tx(
    simulation: Simulation,
    zenoh_router: str,
    zenoh_session: zenoh.Session,
    tmp_path: Path,
) -> None:
    """
    Verify FlexRay data transmission over Zenoh.
    """
    flexray_dir = build_flexray_artifacts()
    dtb_path = Path(flexray_dir) / "platform.dtb"
    kernel_path = Path(flexray_dir) / "firmware.elf"

    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    topic = SimTopic.flexray_unique_prefix(unique_id)

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        "virtmcu-clock,mode=slaved-icount,node=0",
        "-global",
        f"flexray.topic={topic}",
        "-global",
        f"flexray.router={zenoh_router}",
        "-global",
        "flexray.debug=true",
    ]

    tx_topic = f"{topic}/0/tx"
    queue: asyncio.Queue[Any] = asyncio.Queue()
    loop = asyncio.get_running_loop()

    def on_msg(sample: zenoh.Sample) -> None:
        loop.call_soon_threadsafe(queue.put_nowait, sample)

    # Declare subscriber BEFORE entering the simulation context — the
    # framework's routing barrier only covers subs declared before __aenter__.
    _sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(tx_topic, on_msg))

    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        # Run for 20ms virtual time
        for _ in range(100):
            await sim.vta.step(1_000_000)
            if not queue.empty():
                break

        assert not queue.empty(), "No FlexRay frames received over Zenoh"
        sample = queue.get_nowait()
        assert b"\xde\xad\xc0\xde" in sample.payload.to_bytes()


@pytest.mark.asyncio
async def test_flexray_zenoh_rx(
    simulation: Simulation,
    zenoh_router: str,
    zenoh_session: zenoh.Session,
    tmp_path: Path,
) -> None:
    """
    Verify FlexRay data reception from Zenoh.
    """
    flexray_dir = build_flexray_artifacts()
    dtb_path = Path(flexray_dir) / "platform.dtb"
    kernel_path = Path(flexray_dir) / "firmware.elf"

    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    topic = SimTopic.flexray_unique_prefix(unique_id)
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
        "-global",
        "flexray.debug=true",
    ]

    import flatbuffers
    from virtmcu.flexray import FlexRayFrame

    # Declare publisher BEFORE entering the simulation context.
    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(rx_topic))

    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        builder = flatbuffers.Builder(1024)
        data_off = builder.CreateByteVector(b"\xef\xbe\xad\xde")
        FlexRayFrame.Start(builder)
        FlexRayFrame.AddFrameId(builder, 20)
        FlexRayFrame.AddData(builder, data_off)
        FlexRayFrame.AddDeliveryVtimeNs(builder, 5_000_000)
        frame_off = FlexRayFrame.End(builder)
        builder.Finish(frame_off)

        await asyncio.to_thread(partial(pub.put, builder.Output()))

        for _ in range(100):
            await sim.vta.step(1_000_000)
            assert sim.bridge is not None
            if b"\xef\xbe\xad\xde" in sim.bridge.uart_buffer_raw:
                break
        # Check UART
        assert sim.bridge is not None
        uart_data = sim.bridge.uart_buffer_raw
        assert b"\xef\xbe\xad\xde" in uart_data


@pytest.mark.asyncio
async def test_flexray_stress(
    simulation: Simulation,
    zenoh_router: str,
    zenoh_session: zenoh.Session,
    tmp_path: Path,
) -> None:
    """
    Verify FlexRay controller under heavy load.
    """
    flexray_dir = build_flexray_artifacts()
    dtb_path = Path(flexray_dir) / "platform.dtb"
    kernel_path = Path(flexray_dir) / "firmware.elf"

    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    topic = SimTopic.flexray_unique_prefix(unique_id)
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
        "-global",
        "flexray.debug=true",
    ]

    import flatbuffers
    from virtmcu.flexray import FlexRayFrame

    # Declare publisher BEFORE entering the simulation context.
    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(rx_topic))

    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        for i in range(100):
            builder = flatbuffers.Builder(64)
            data_off = builder.CreateByteVector(b"STRESS")
            FlexRayFrame.Start(builder)
            FlexRayFrame.AddFrameId(builder, 20)
            FlexRayFrame.AddData(builder, data_off)
            FlexRayFrame.AddDeliveryVtimeNs(builder, 1_000_000 + (i * 10_000))
            frame_off = FlexRayFrame.End(builder)
            builder.Finish(frame_off)
            await asyncio.to_thread(partial(pub.put, builder.Output()))

        await sim.vta.run_for(50_000_000)

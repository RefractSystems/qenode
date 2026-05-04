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
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest

from tools.testing.virtmcu_test_suite.factory import compile_yaml
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from typing import Any

    from tools.testing.virtmcu_test_suite.simulation import Simulation



logger = logging.getLogger(__name__)


@pytest.fixture
def flexray_artifacts(guest_app_factory: Callable[[str], Path], tmp_path: Path) -> tuple[Path, Path]:
    app_dir = guest_app_factory("flexray_bridge")
    
    yaml_src = """
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals:
  - name: ram
    type: Memory.MappedMemory
    address: 0x40000000
    properties:
      size: 0x08000000
  - name: flexray
    type: flexray
    address: 0x09003000
  - name: pl011
    type: pl011
    address: 0x09000000
    properties:
      chardev: 0
"""
    yaml_path = tmp_path / "platform.yaml"
    yaml_path.write_text(yaml_src)
    dtb_path = compile_yaml(yaml_path, tmp_path / "platform.dtb")
    kernel_path = app_dir / "firmware.elf"
    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_flexray_zenoh_tx(
    simulation: Simulation,
    tmp_path: Path,
    flexray_artifacts: tuple[Path, Path],
) -> None:
    """
    Verify FlexRay data transmission over Zenoh.
    """
    dtb_path, kernel_path = flexray_artifacts

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
        f"flexray.router={simulation._router}",
        "-global",
        "flexray.debug=true",
    ]

    tx_topic = f"{topic}/0/tx"
    queue: asyncio.Queue[Any] = asyncio.Queue()
    loop = asyncio.get_running_loop()

    if simulation.transport is None:
        from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl
        simulation.transport = ZenohTransportImpl(simulation._router, simulation._session)
    assert simulation.transport is not None
    def on_msg(payload: bytes) -> None:
        loop.call_soon_threadsafe(queue.put_nowait, payload)

    # Declare subscriber BEFORE entering the simulation context — the
    # framework's routing barrier only covers subs declared before __aenter__.
    await simulation.transport.subscribe(tx_topic, on_msg)

    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        # Run for 20ms virtual time
        for _ in range(100):
            await sim.vta.step(1_000_000)  # LINT_EXCEPTION: vta_step_loop
            if not queue.empty():
                break

        assert not queue.empty(), "No FlexRay frames received over Zenoh"
        sample = queue.get_nowait()
        assert b"\xde\xad\xc0\xde" in sample


@pytest.mark.asyncio
async def test_flexray_zenoh_rx(
    simulation: Simulation,
    tmp_path: Path,
    flexray_artifacts: tuple[Path, Path],
) -> None:
    """
    Verify FlexRay data reception from Zenoh.
    """
    dtb_path, kernel_path = flexray_artifacts

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
        f"flexray.router={simulation._router}",
        "-global",
        "flexray.debug=true",
    ]

    import flatbuffers

    from tools.flexray_fbs.virtmcu.flexray import FlexRayFrame

    # Declare publisher BEFORE entering the simulation context.
    if simulation.transport is None:
        from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl
        simulation.transport = ZenohTransportImpl(simulation._router, simulation._session)
    assert simulation.transport is not None

    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        builder = flatbuffers.Builder(1024)
        data_off = builder.CreateByteVector(b"\xef\xbe\xad\xde")
        FlexRayFrame.Start(builder)
        FlexRayFrame.AddFrameId(builder, 20)  # type: ignore
        FlexRayFrame.AddData(builder, data_off)  # type: ignore
        FlexRayFrame.AddDeliveryVtimeNs(builder, 5_000_000)  # type: ignore
        frame_off = FlexRayFrame.End(builder)
        builder.Finish(frame_off)

        asyncio.create_task(simulation.transport.publish(rx_topic, builder.Output()))

        for _ in range(100):
            await sim.vta.step(1_000_000)  # LINT_EXCEPTION: vta_step_loop
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
    tmp_path: Path,
    flexray_artifacts: tuple[Path, Path],
) -> None:
    """
    Verify FlexRay controller under heavy load.
    """
    dtb_path, kernel_path = flexray_artifacts

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
        f"flexray.router={simulation._router}",
        "-global",
        "flexray.debug=true",
    ]

    import flatbuffers

    from tools.flexray_fbs.virtmcu.flexray import FlexRayFrame

    # Declare publisher BEFORE entering the simulation context.
    if simulation.transport is None:
        from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl
        simulation.transport = ZenohTransportImpl(simulation._router, simulation._session)
    assert simulation.transport is not None

    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=extra_args)
    async with simulation as sim:
        for i in range(100):
            builder = flatbuffers.Builder(64)
            data_off = builder.CreateByteVector(b"STRESS")
            FlexRayFrame.Start(builder)
            FlexRayFrame.AddFrameId(builder, 20)  # type: ignore
            FlexRayFrame.AddData(builder, data_off)  # type: ignore
            FlexRayFrame.AddDeliveryVtimeNs(builder, 1_000_000 + (i * 10_000))  # type: ignore
            frame_off = FlexRayFrame.End(builder)
            builder.Finish(frame_off)
            asyncio.create_task(simulation.transport.publish(rx_topic, builder.Output()))

        await sim.vta.run_for(50_000_000)

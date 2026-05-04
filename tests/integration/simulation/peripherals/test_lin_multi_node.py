"""
SOTA Test Module: test_lin_multi_node

Context:
This module implements tests for the test_lin_multi_node subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_lin_multi_node.
"""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:

    from tools.testing.virtmcu_test_suite.simulation import Simulation


from tools.lin_fbs.virtmcu.lin import LinFrame, LinMessageType
from tools.testing.virtmcu_test_suite.factory import compile_dtb, compile_firmware

logger = logging.getLogger(__name__)


@pytest.fixture
def lin_master_elf(tmp_path: Path) -> Path:
    out = tmp_path / "lin_master.elf"
    compile_firmware(
        [Path("tests/fixtures/guest_apps/lin_bridge/lin_master.S")],
        out,
        linker_script=Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.ld"),
    )
    return out


@pytest.fixture
def lin_slave_elf(tmp_path: Path) -> Path:
    out = tmp_path / "lin_slave.elf"
    compile_firmware(
        [Path("tests/fixtures/guest_apps/lin_bridge/lin_slave.S")],
        out,
        linker_script=Path("tests/fixtures/guest_apps/lin_bridge/lin_echo.ld"),
    )
    return out


@pytest.fixture
def lin_master_dtb(tmp_path: Path, zenoh_router: str) -> Path:
    out = tmp_path / "lin_master.dtb"
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {"ZENOH_ROUTER_ENDPOINT": zenoh_router},
        out,
    )
    return out


@pytest.fixture
def lin_slave_dtb(tmp_path: Path, zenoh_router: str) -> Path:
    out = tmp_path / "lin_slave.dtb"
    compile_dtb(
        Path("tests/fixtures/guest_apps/lin_bridge/lin_test.dts"),
        {
            "node = <0>;": "node = <1>;",
            "ZENOH_ROUTER_ENDPOINT": zenoh_router,
        },
        out,
    )
    return out


@pytest.mark.asyncio
@pytest.mark.timeout(400)
@pytest.mark.parametrize(
    "deterministic_coordinator",
    [{"nodes": 2, "pdes": True, "topology": "tests/fixtures/topologies/lin_2node.yml"}],
    indirect=True,
)
@pytest.mark.usefixtures("deterministic_coordinator")
async def test_multi_node_lin(
    simulation: Simulation,
    lin_master_elf: Path,
    lin_slave_elf: Path,
    lin_master_dtb: Path,
    lin_slave_dtb: Path,
) -> None:
    master_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n0",
        "-serial",
        "chardev:n0",
        "-net",
        "none",
        "-device",
        "virtmcu-clock,coordinated=true",
    ]

    slave_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n1",
        "-serial",
        "chardev:n1",
        "-net",
        "none",
        "-device",
        "virtmcu-clock,coordinated=true",
    ]

    simulation.add_node(node_id=0, dtb=lin_master_dtb, kernel=lin_master_elf, extra_args=master_args)
    simulation.add_node(node_id=1, dtb=lin_slave_dtb, kernel=lin_slave_elf, extra_args=slave_args)

    assert simulation.transport is not None

    bus_messages: list[tuple[str, int, bytes]] = []


    def on_bus_msg(topic: str, payload: bytes) -> None:
        try:
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])  # type: ignore[misc]
            logger.info(f"Bus: {topic} type={msg_type} data={data!r}")
            bus_messages.append((topic, msg_type, data))
        except Exception:  # noqa: BLE001
            logger.error("Ignored")

    from tools.testing.virtmcu_test_suite.topics import SimTopic

    def on_msg_0(payload: bytes) -> None:
        on_bus_msg(SimTopic.lin_tx(0), payload)

    def on_msg_1(payload: bytes) -> None:
        on_bus_msg(SimTopic.lin_tx(1), payload)

    condition_event = asyncio.Event()

    def check_condition() -> None:
        found_master_header = False
        found_slave_response = False
        for topic, msg_type, data in bus_messages:
            if topic.endswith("/0/tx") and msg_type == LinMessageType.LinMessageType.Break:
                found_master_header = True
            if topic.endswith("/1/tx") and msg_type == LinMessageType.LinMessageType.Data and b"S" in data:
                found_slave_response = True
        if found_master_header and found_slave_response:
            condition_event.set()

    # Update the callbacks to check the condition
    def on_msg_0_event(payload: bytes) -> None:
        on_msg_0(payload)
        check_condition()

    def on_msg_1_event(payload: bytes) -> None:
        on_msg_1(payload)
        check_condition()

    await simulation.transport.subscribe(SimTopic.lin_tx(0), on_msg_0_event)
    await simulation.transport.subscribe(SimTopic.lin_tx(1), on_msg_1_event)

    async with simulation as sim:
        logger.info("Launching Master and Slave via Simulation...")

        # Wait passively for the event. The deterministic coordinator handles the clock.
        try:
            await sim.run_until(condition_event.is_set, timeout_ns=1_000_000_000, step_ns=1_000_000, timeout=120.0)
        except TimeoutError:
            pytest.fail("LIN multi-node communication failed to complete within virtual timeout")

        logger.info("SUCCESS: Multi-node LIN communication verified")

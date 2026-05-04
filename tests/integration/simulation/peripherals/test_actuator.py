"""
SOTA Test Module: test_actuator

Context:
This module implements tests for the test_actuator subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_actuator.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING, Any

import pytest

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.simulation import Simulation


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_actuator_zenoh_publish(
    simulation: Simulation,
    tmp_path: Path,
    guest_app_factory: Any,  # noqa: ANN401
) -> None:
    """
    Test that the actuator device correctly publishes to Zenoh.
    """
    from tools.testing.virtmcu_test_suite.factory import compile_yaml

    app_dir = guest_app_factory("actuator")
    yaml_file = app_dir / "board.yaml"
    tmp_yaml = tmp_path / "board.yaml"
    dtb = tmp_path / "board.dtb"
    kernel = app_dir / "actuator.elf"

    # Copy and substitute the router endpoint in the YAML
    yaml_content = yaml_file.read_text().replace("ZENOH_ROUTER_ENDPOINT", simulation._router)
    tmp_yaml.write_text(yaml_content)

    compile_yaml(tmp_yaml, dtb)

    received_msgs: list[dict[str, Any]] = []

    def on_sample(topic: str, payload: bytes) -> None:
        if len(payload) < 8:
            return
        vtime_ns = int.from_bytes(payload[:8], "little")
        data_bytes = payload[8:]
        import array

        a = array.array("d", data_bytes)
        vals = a.tolist()
        received_msgs.append({"topic": topic, "vtime": vtime_ns, "vals": vals})

    extra_args = [
        "-icount",
        "shift=4,align=off,sleep=off",
        "-device",
        "virtmcu-clock,node=0,mode=slaved-icount",
    ]

    # Declare subscribers BEFORE entering the simulation context so the
    # framework's routing barrier covers them.
    if simulation.transport is None:
        from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl

        simulation.transport = ZenohTransportImpl(simulation._router, simulation._session)
    assert simulation.transport is not None

    await simulation.transport.subscribe("firmware/control/0/42", lambda p: on_sample("firmware/control/0/42", p))
    await simulation.transport.subscribe("firmware/control/0/99", lambda p: on_sample("firmware/control/0/99", p))

    simulation.add_node(node_id=0, dtb=dtb, kernel=kernel, extra_args=extra_args)
    async with simulation as sim:
        success_1 = False
        success_2 = False

        # Advance until we get messages
        for _ in range(50):
            await sim.vta.step(10_000_000)  # LINT_EXCEPTION: vta_step_loop
            for msg in received_msgs:
                if msg["topic"] == "firmware/control/0/42" and abs(msg["vals"][0] - 3.14) < 0.001:
                    success_1 = True
                elif (
                    msg["topic"] == "firmware/control/0/99" and len(msg["vals"]) == 3 and msg["vals"] == [1.0, 2.0, 3.0]
                ):
                    success_2 = True
            if success_1 and success_2:
                break
        else:
            pytest.fail(
                f"Did not receive all control signals (s1={success_1}, s2={success_2}) at vtime={sim.vta.current_vtimes[0]}"
            )

        assert success_1, "Did not receive first control signal (ID=42)"
        assert success_2, "Did not receive second control signal (ID=99)"

"""
SOTA Test Module: test_actuator

Context:
This module implements tests for the test_actuator subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_actuator.
"""

import logging
from typing import Any

import pytest

logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_actuator_zenoh_publish(simulation, zenoh_router, tmp_path):
    """
    Test that the actuator device correctly publishes to Zenoh.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT

    yaml_file = workspace_root / "tests/fixtures/guest_apps/actuator/board.yaml"
    tmp_yaml = tmp_path / "board.yaml"
    dtb = tmp_path / "board.dtb"
    kernel = workspace_root / "tests/fixtures/guest_apps/actuator/actuator.elf"

    if not kernel.exists():
        import subprocess

        subprocess.run(["make", "-C", "tests/fixtures/guest_apps/actuator"], check=True, cwd=workspace_root)

    # Copy and substitute the router endpoint in the YAML
    yaml_content = yaml_file.read_text().replace("ZENOH_ROUTER_ENDPOINT", zenoh_router)
    tmp_yaml.write_text(yaml_content)

    import subprocess

    subprocess.run(
        ["uv", "run", "python3", "-m", "tools.yaml2qemu", str(tmp_yaml), "--out-dtb", str(dtb)],
        check=True,
        cwd=workspace_root,
    )

    received_msgs: list[dict[str, Any]] = []

    def on_sample(sample):
        topic = str(sample.key_expr)
        payload = sample.payload.to_bytes()
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

    async with await simulation(dtb, kernel, nodes=[0], extra_args=extra_args, ignore_clock_check=True) as sim:
        sim.vta.session.declare_subscriber("firmware/control/**", on_sample)

        success_1 = False
        success_2 = False

        # Advance until we get messages
        for _ in range(50):
            await sim.vta.step(10_000_000)
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

import asyncio
import struct
from pathlib import Path

import pytest


@pytest.mark.xdist_group(name="serial-clock")
@pytest.mark.asyncio
async def test_actuator_zenoh_publish(qemu_launcher, zenoh_router, zenoh_session):
    """
    Test that the zenoh-actuator device correctly publishes to Zenoh.
    """
    workspace_root = Path(__file__).resolve().parent.parent
    dtb = workspace_root / "test/actuator/board.dtb"
    kernel = workspace_root / "test/actuator/actuator.elf"

    # 1. Build if missing
    if not dtb.exists() or not kernel.exists():
        import subprocess

        subprocess.run(["make", "-C", "test/actuator"], check=True, cwd=workspace_root)

    received_msgs = []

    def on_sample(sample):
        topic = str(sample.key_expr)
        payload = sample.payload.to_bytes()
        # Use print to stderr to be sure it's seen
        import sys

        print(f"DEBUG: Received Zenoh msg on topic: {topic}, len={len(payload)}", file=sys.stderr)
        if len(payload) < 8:
            return
        vtime_ns = struct.unpack("<Q", payload[:8])[0]
        data_bytes = payload[8:]
        n_doubles = len(data_bytes) // 8
        vals = struct.unpack("<" + "d" * n_doubles, data_bytes)
        received_msgs.append({"topic": topic, "vtime": vtime_ns, "vals": vals})

    zenoh_session.declare_subscriber("firmware/control/**", on_sample)

    bridge = await qemu_launcher(
        dtb, kernel, extra_args=["-global", f"zenoh-actuator.router={zenoh_router}"], ignore_clock_check=True
    )

    await bridge.start_emulation()

    # Wait for messages
    timeout = 15.0
    start_time = asyncio.get_event_loop().time()

    success_1 = False
    success_2 = False

    while asyncio.get_event_loop().time() - start_time < timeout:
        for msg in received_msgs:
            # Topic should be firmware/control/42 and firmware/control/99
            if msg["topic"] == "firmware/control/42" and abs(msg["vals"][0] - 3.14) < 0.001:
                success_1 = True
            elif msg["topic"] == "firmware/control/99" and len(msg["vals"]) == 3 and msg["vals"] == (1.0, 2.0, 3.0):
                success_2 = True

        if success_1 and success_2:
            break

        await asyncio.sleep(0.5)

    assert success_1, "Did not receive first control signal (ID=42)"
    assert success_2, "Did not receive second control signal (ID=99)"

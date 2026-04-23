import struct
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_actuator_zenoh_publish(qemu_launcher, zenoh_router, zenoh_session, tmp_path):
    """
    Test that the zenoh-actuator device correctly publishes to Zenoh.
    """
    workspace_root = Path(__file__).resolve().parent.parent

    yaml_file = workspace_root / "test/actuator/board.yaml"
    tmp_yaml = tmp_path / "board.yaml"
    dtb = tmp_path / "board.dtb"
    kernel = workspace_root / "test/actuator/actuator.elf"

    if not kernel.exists():
        import subprocess

        subprocess.run(["make", "-C", "test/actuator"], check=True, cwd=workspace_root)

    import subprocess

    # Copy and substitute the router endpoint in the YAML
    yaml_content = yaml_file.read_text().replace("tcp/127.0.0.1:7450", zenoh_router)
    tmp_yaml.write_text(yaml_content)

    subprocess.run(
        ["uv", "run", "python3", "-m", "tools.yaml2qemu", str(tmp_yaml), "--out-dtb", str(dtb)],
        check=True,
        cwd=workspace_root,
    )

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

    extra_args = [
        "-icount", "shift=4,align=off,sleep=off",
        "-device", f"zenoh-clock,node=0,mode=slaved-icount,router={zenoh_router}",
    ]
    bridge = await qemu_launcher(dtb, kernel, extra_args=extra_args, ignore_clock_check=True)

    from tests.conftest import wait_for_zenoh_discovery
    await wait_for_zenoh_discovery(zenoh_session, "firmware/control/**")

    await bridge.start_emulation()

    from tests.conftest import VirtualTimeAuthority
    vta = VirtualTimeAuthority(zenoh_session, [0])

    success_1 = False
    success_2 = False

    # Advance until we get messages
    for _ in range(50):
        await vta.step(10_000_000)
        for msg in received_msgs:
            # Topic should be firmware/control/0/42 and firmware/control/0/99
            if msg["topic"] == "firmware/control/0/42" and abs(msg["vals"][0] - 3.14) < 0.001:
                success_1 = True
            elif msg["topic"] == "firmware/control/0/99" and len(msg["vals"]) == 3 and msg["vals"] == (1.0, 2.0, 3.0):
                success_2 = True
        if success_1 and success_2:
            break
    else:
        pytest.fail(f"Did not receive all control signals (s1={success_1}, s2={success_2}) at vtime={vta.current_vtimes[0]}")

    assert success_1, "Did not receive first control signal (ID=42)"
    assert success_2, "Did not receive second control signal (ID=99)"

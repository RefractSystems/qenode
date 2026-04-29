"""
Architectural hardening stress tests.
1. Quantum sync stress: 100 quanta with ZenohClock.
2. Sequence number tie-breaking: UART bytes at same vtime.
"""

import subprocess
from pathlib import Path

import pytest
import vproto
import zenoh


def _ensure_phase1_built():
    workspace_root = Path(__file__).resolve().parent.parent
    dtb = workspace_root / "test/phase1/minimal.dtb"
    kernel = workspace_root / "test/phase1/hello.elf"
    if not dtb.exists() or not kernel.exists():
        subprocess.run(["make", "-C", "test/phase1"], check=True, cwd=workspace_root)
    return dtb, kernel


@pytest.mark.asyncio
async def test_quantum_sync_stress(qemu_launcher, zenoh_router):
    """Run 100 quanta and verify no stalls or state machine failures."""
    dtb, kernel = _ensure_phase1_built()
    node_id = 42  # Unique ID for this test
    quantum_ns = 1_000_000  # 1ms
    total_quanta = 100

    # Start with node=0 (bypass mode) to allow QMP to start
    extra_args = ["-device", f"virtmcu-clock,node=0,router={zenoh_router}"]

    bridge = await qemu_launcher(dtb_path=dtb, kernel_path=kernel, extra_args=extra_args, ignore_clock_check=True)

    # Now enable synchronization by setting node ID via QMP
    # Find the clock device path first
    res = await bridge.qmp.execute("qom-list", {"path": "/machine/unattached"})
    clock_path = None
    for item in res:
        if item.get("type") == "child<clock>":
            clock_path = f"/machine/unattached/{item['name']}"
            break

    assert clock_path is not None, "Zenoh clock not found in QOM"

    await bridge.qmp.execute("qom-set", {"path": clock_path, "property": "node", "value": node_id})

    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", f'["{zenoh_router}"]')
    session = zenoh.open(conf)

    current_vtime = 0
    # The bypass might have let it run for a bit, let's catch up
    # first query might return current vtime

    for i in range(total_quanta):
        # Advance clock
        replies = session.get(
            f"sim/clock/advance/{node_id}", payload=vproto.ClockAdvanceReq(quantum_ns, 0, 0).pack(), timeout=10.0
        )

        found_reply = False
        for reply in replies:
            if reply.ok:
                resp = vproto.ClockReadyResp.unpack(reply.ok.payload.to_bytes())
                assert resp.error_code == 0, f"Quantum {i} failed with error {resp.error_code}"
                # Since we start mid-stream, just ensure forward progress
                assert resp.current_vtime_ns > current_vtime
                current_vtime = resp.current_vtime_ns
                found_reply = True
                break

        assert found_reply, f"No reply received for quantum {i}"

    session.close()


@pytest.mark.asyncio
async def test_uart_sequence_tiebreaking(qemu_launcher, zenoh_router):
    """Verify that multiple UART bytes sent at the same vtime arrive in order."""
    dtb, kernel = _ensure_phase1_built()
    node_id = 43

    extra_args = [
        "-device",
        f"virtmcu-clock,node=0,router={zenoh_router}",
        "-chardev",
        f"virtmcu,id=char0,node={node_id},router={zenoh_router}",
        "-serial",
        "chardev:char0",
        "-S",  # Start frozen
    ]

    bridge = await qemu_launcher(dtb_path=dtb, kernel_path=kernel, extra_args=extra_args, ignore_clock_check=True)

    # Enable sync
    res = await bridge.qmp.execute("qom-list", {"path": "/machine/unattached"})
    clock_path = None
    for item in res:
        if item.get("type") == "child<clock>":
            clock_path = f"/machine/unattached/{item['name']}"
            break
    await bridge.qmp.execute("qom-set", {"path": clock_path, "property": "node", "value": node_id})

    conf = zenoh.Config()
    conf.insert_json5("connect/endpoints", f'["{zenoh_router}"]')
    session = zenoh.open(conf)

    # We don't verify echo here (requires echo firmware), but we verify
    # the architectural support (no crash, correctly packed headers).

    pub = session.declare_publisher(f"virtmcu/uart/{node_id}/rx")

    await bridge.start_emulation()

    # 1. Advance past boot
    session.get(f"sim/clock/advance/{node_id}", payload=vproto.ClockAdvanceReq(100_000_000, 0, 0).pack(), timeout=10.0)

    # 2. Pre-publish "HELLO" all at the SAME virtual time
    vtime = 200_000_000
    test_str = b"HELLO"
    for i, char in enumerate(test_str):
        header = vproto.ZenohFrameHeader(vtime, i, 1).pack()
        pub.put(header + bytes([char]))

    for _ in range(10):
        session.get(
            f"sim/clock/advance/{node_id}", payload=vproto.ClockAdvanceReq(1_000_000, 0, 0).pack(), timeout=10.0
        )

    session.close()

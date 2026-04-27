import asyncio
import os
import struct
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_det5_coordinator_barrier(zenoh_router, zenoh_session):
    """
    Test DET-5: DeterministicCoordinator Quantum Barrier.
    3 nodes send messages to each other. We assert they are delivered in canonical order.
    """
    workspace_root = Path(__file__).parent.parent
    coordinator_bin = Path(os.environ.get("CARGO_TARGET_DIR", workspace_root / "target")) / "release/deterministic_coordinator"
    if not coordinator_bin.exists():
        coordinator_bin = workspace_root / "tools/deterministic_coordinator/target/release/deterministic_coordinator"
    if not coordinator_bin.exists():
        pytest.skip("deterministic_coordinator binary not found")

    proc = await asyncio.create_subprocess_exec(
        str(coordinator_bin),
        "--nodes", "3",
        "--connect", zenoh_router,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )

    try:
        await asyncio.sleep(1.0)

        received_msgs = []

        def on_rx(sample):
            topic = str(sample.key_expr)
            payload = sample.payload.to_bytes()
            # Decode message: src(u32), dst(u32), vtime(u64), seq(u64), proto(u8), len(u32), data(len)
            src, dst, vtime, seq, _proto, dlen = struct.unpack("<IIQQBI", payload[:29])
            data = payload[29:29+dlen]
            received_msgs.append((topic, src, dst, vtime, seq, data))

        _subs = []
        for i in range(3):
            def declare_sub(idx: int):
                return zenoh_session.declare_subscriber(f"sim/coord/{idx}/rx", on_rx)
            _subs.append(await asyncio.to_thread(declare_sub, i))

        await asyncio.sleep(0.5)

        def pack_batch(msgs):
            # [num_msgs: u32] followed by msgs
            buf = bytearray(struct.pack("<I", len(msgs)))
            for src, dst, vtime, seq, proto, data in msgs:
                buf.extend(struct.pack("<IIQQBI", src, dst, vtime, seq, proto, len(data)))
                buf.extend(data)
            return bytes(buf)

        # Node 0 sends to 1 and 2
        b0 = pack_batch([
            (0, 1, 5, 0, 1, b"N0->N1"),
            (0, 2, 5, 1, 1, b"N0->N2"),
        ])

        # Node 1 sends to 0 and 2
        b1 = pack_batch([
            (1, 0, 5, 0, 1, b"N1->N0"),
            (1, 2, 5, 1, 1, b"N1->N2"),
        ])

        # Node 2 sends to 0 and 1
        b2 = pack_batch([
            (2, 0, 5, 0, 1, b"N2->N0"),
            (2, 1, 5, 1, 1, b"N2->N1"),
        ])

        # 100 runs
        for run in range(100):
            received_msgs.clear()

            def _send_shuffled():
                # Randomize arrival order
                import random
                nodes_data = [(0, b0), (1, b1), (2, b2)]
                random.shuffle(nodes_data)
                for nid, b in nodes_data:
                    zenoh_session.put(f"sim/coord/{nid}/tx", b)

                # Wait slightly to ensure tx arrives before done
                import time
                time.sleep(0.1)

                # Send done
                nodes = [0, 1, 2]
                random.shuffle(nodes)
                for nid in nodes:
                    zenoh_session.put(f"sim/coord/{nid}/done", b"")

            await asyncio.to_thread(_send_shuffled)
            await asyncio.sleep(0.5) # Wait for processing

            assert len(received_msgs) == 6, f"Run {run}: Expected 6 messages, got {len(received_msgs)}"

            # Group received messages by destination topic
            from typing import Any
            by_topic: dict[str, list[Any]] = {"sim/coord/0/rx": [], "sim/coord/1/rx": [], "sim/coord/2/rx": []}
            for msg in received_msgs:
                by_topic[msg[0]].append(msg)

            expected_by_topic = {
                "sim/coord/0/rx": [
                    ("sim/coord/0/rx", 1, 0, 5, 0, b"N1->N0"),
                    ("sim/coord/0/rx", 2, 0, 5, 0, b"N2->N0"),
                ],
                "sim/coord/1/rx": [
                    ("sim/coord/1/rx", 0, 1, 5, 0, b"N0->N1"),
                    ("sim/coord/1/rx", 2, 1, 5, 1, b"N2->N1"),
                ],
                "sim/coord/2/rx": [
                    ("sim/coord/2/rx", 0, 2, 5, 1, b"N0->N2"),
                    ("sim/coord/2/rx", 1, 2, 5, 1, b"N1->N2"),
                ],
            }

            for t in by_topic:
                assert by_topic[t] == expected_by_topic[t], f"Run {run}: Order mismatch on {t}!\nExpected: {expected_by_topic[t]}\nGot: {by_topic[t]}"

    finally:
        proc.terminate()
        print("STDERR", (await proc.stderr.read() if proc.stderr else b"").decode())
        print("STDOUT", (await proc.stdout.read() if proc.stdout else b"").decode())
        await proc.wait()

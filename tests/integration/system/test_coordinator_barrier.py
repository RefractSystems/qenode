"""
SOTA Test Module: test_coordinator_barrier

Context:
This module implements tests for the test_coordinator_barrier subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator_barrier.
"""

from __future__ import annotations

import asyncio
import logging
from typing import TYPE_CHECKING, Any

import pytest
import zenoh

from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary
from tools.testing.virtmcu_test_suite.conftest_core import coordinator_subprocess
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    pass


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_coordinator_barrier(zenoh_router: str, zenoh_session: zenoh.Session) -> None:
    """
    Test DeterministicCoordinator Quantum Barrier.
    3 nodes send messages to each other. We assert they are delivered in canonical order.
    """
    coordinator_bin = resolve_rust_binary(VirtmcuBinary.DETERMINISTIC_COORDINATOR)

    received_msgs: list[tuple[str, int, int, int, int, bytes]] = []

    def on_rx(sample: zenoh.Sample) -> None:
        topic = str(sample.key_expr)
        payload = sample.payload.to_bytes()
        # Decode message: src(u32), dst(u32), vtime(u64), seq(u64), proto(u8), len(u32), data(len)
        src = int.from_bytes(payload[0:4], "little")
        dst = int.from_bytes(payload[4:8], "little")
        vtime = int.from_bytes(payload[8:16], "little")
        seq = int.from_bytes(payload[16:24], "little")
        _proto = payload[24]
        dlen = int.from_bytes(payload[25:29], "little")
        data = payload[29 : 29 + dlen]
        received_msgs.append((topic, src, dst, vtime, seq, data))

    # Declare subscribers BEFORE entering the coordinator context — the
    # framework's routing barrier covers them.
    _subs = []
    for i in range(3):

        def declare_sub(idx: int) -> object:
            return zenoh_session.declare_subscriber(SimTopic.coord_rx(idx), on_rx)

        _subs.append(await asyncio.to_thread(declare_sub, i))

    async with coordinator_subprocess(
        binary=coordinator_bin,
        args=["--nodes", "3", "--connect", zenoh_router],
        zenoh_session=zenoh_session,
        liveliness_topic=SimTopic.COORD_ALIVE,
    ):

        def pack_batch(msgs: list[tuple[int, int, int, int, int, bytes]]) -> bytes:
            # [num_msgs: u32] followed by msgs
            buf = bytearray(len(msgs).to_bytes(4, "little"))
            for src, dst, vtime, seq, proto, data in msgs:
                buf.extend(
                    src.to_bytes(4, "little")
                    + dst.to_bytes(4, "little")
                    + vtime.to_bytes(8, "little")
                    + seq.to_bytes(8, "little")
                    + proto.to_bytes(1, "little")
                    + len(data).to_bytes(4, "little")
                )
                buf.extend(data)

            return bytes(buf)

        # Node 0 sends to 1 and 2
        b0 = pack_batch(
            [
                (0, 1, 5, 0, 1, b"N0->N1"),
                (0, 2, 5, 1, 1, b"N0->N2"),
            ]
        )

        # Node 1 sends to 0 and 2
        b1 = pack_batch(
            [
                (1, 0, 5, 0, 1, b"N1->N0"),
                (1, 2, 5, 1, 1, b"N1->N2"),
            ]
        )

        # Node 2 sends to 0 and 1
        b2 = pack_batch(
            [
                (2, 0, 5, 0, 1, b"N2->N0"),
                (2, 1, 5, 1, 1, b"N2->N1"),
            ]
        )

        # 100 runs
        for run in range(1, 101):
            received_msgs.clear()

            def _send_shuffled(q: int) -> None:
                # Randomize arrival order
                import random

                nodes = [0, 1, 2]
                batches = {0: b0, 1: b1, 2: b2}
                random.shuffle(nodes)
                for nid in nodes:
                    q_payload = q.to_bytes(8, "little") + batches[nid]
                    zenoh_session.put(SimTopic.coord_done(nid), q_payload)

            await asyncio.to_thread(_send_shuffled, run)

            # Wait for messages to be delivered with timeout
            import time

            start_wait = time.time()
            while len(received_msgs) < 6 and time.time() - start_wait < 5.0:
                await asyncio.sleep(0.05)  # SLEEP_EXCEPTION: deterministic polling for message delivery

            assert len(received_msgs) == 6, f"Run {run}: Expected 6 messages, got {len(received_msgs)} within 5s"

            # Group received messages by destination topic
            by_topic: dict[str, list[Any]] = {
                SimTopic.coord_rx(0): [],
                SimTopic.coord_rx(1): [],
                SimTopic.coord_rx(2): [],
            }
            for msg in received_msgs:
                by_topic[msg[0]].append(msg)

            expected_by_topic = {
                SimTopic.coord_rx(0): [
                    (SimTopic.coord_rx(0), 1, 0, 5, 0, b"N1->N0"),
                    (SimTopic.coord_rx(0), 2, 0, 5, 0, b"N2->N0"),
                ],
                SimTopic.coord_rx(1): [
                    (SimTopic.coord_rx(1), 0, 1, 5, 0, b"N0->N1"),
                    (SimTopic.coord_rx(1), 2, 1, 5, 1, b"N2->N1"),
                ],
                SimTopic.coord_rx(2): [
                    (SimTopic.coord_rx(2), 0, 2, 5, 1, b"N0->N2"),
                    (SimTopic.coord_rx(2), 1, 2, 5, 1, b"N1->N2"),
                ],
            }

            for t in by_topic:
                assert by_topic[t] == expected_by_topic[t], (
                    f"Run {run}: Order mismatch on {t}!\nExpected: {expected_by_topic[t]}\nGot: {by_topic[t]}"
                )

        # 4. Zero message quantum test
        received_msgs.clear()
        q_num = 101
        q_payload = q_num.to_bytes(8, "little")
        for i in range(3):
            zenoh_session.put(SimTopic.coord_done(i), q_payload)

        assert len(received_msgs) == 0, "Expected no messages in zero-message quantum"

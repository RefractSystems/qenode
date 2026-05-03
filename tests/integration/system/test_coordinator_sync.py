"""
SOTA Test Module: test_coordinator_sync

Context:
This module implements tests for the test_coordinator_sync subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator_sync.
"""

from __future__ import annotations

import asyncio
import contextlib
import os
import time
import typing
from typing import TYPE_CHECKING, Any, cast

import pytest
import zenoh

from tools.testing.utils import get_time_multiplier
from tools.testing.virtmcu_test_suite.artifact_resolver import get_rust_binary_path
from tools.testing.virtmcu_test_suite.conftest_core import coordinator_subprocess
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from pathlib import Path

    import zenoh

    from tools.testing.virtmcu_test_suite.simulation import Simulation


_base_stall_timeout_ms = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
_STALL_TIMEOUT_MS = int(_base_stall_timeout_ms * get_time_multiplier())
# VTA timeout must exceed the QEMU stall timeout so QEMU can reply before Python gives up.
_VTA_TIMEOUT_S: float = max(30.0, _STALL_TIMEOUT_MS / 1000.0 + 10.0)

# Minimum wall-clock delay the coordinator must introduce before releasing nodes.
_COORDINATOR_DELIVERY_DELAY_S: float = 0.1


@pytest.mark.asyncio
async def test_coordinator_sync(
    simulation: Simulation,
    tmp_path: Path,
) -> None:
    """
    TA/Coordinator Synchronization Protocol.

    Verifies that the TimeAuthority cannot advance a node to quantum Q+1 before
    the Coordinator has fully delivered all messages from quantum Q.  The
    Python coordinator introduces a deliberate 100 ms delivery delay; the
    assertion checks that the VTA step wall-clock time reflects this delay.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    firmware_path = workspace_root / "tests/fixtures/guest_apps/uart_echo/echo.elf"
    if not firmware_path.exists():
        pytest.fail("echo.elf not found — run 'make -C tests/fixtures/guest_apps/uart_echo' first")

    from tools.testing.virtmcu_test_suite.world_schema import (
        MachineSpec,
        WorldYaml,
    )

    board_yaml = tmp_path / "board.yaml"
    world = WorldYaml(
        machine=MachineSpec(
            cpus=[{"name": "cpu0", "type": "cortex-a15"}],
        ),
        peripherals=[
            {
                "name": "uart0",
                "type": "pl011",
                "address": "0x09000000",
                "interrupt": 4,
            }
        ],
    )
    # Adding 'memory' as a top-level list to match the original structure's flexibility
    # The current WorldYaml.peripherals handles this.
    world.memory = [
        {
            "name": "sram",
            "address": "0x40000000",
            "size": "0x1000000",
        }
    ]
    board_yaml.write_text(world.to_yaml())

    clock_args_n1 = [
        "virtmcu-clock,coordinated=true",
        "-chardev",
        "virtmcu,id=chr1,topic=sim/uart",
        "-serial",
        "chardev:chr1",
    ]
    clock_args_n2 = [
        "virtmcu-clock,coordinated=true",
        "-chardev",
        "virtmcu,id=chr2,topic=sim/uart",
        "-serial",
        "chardev:chr2",
    ]

    simulation.add_node(node_id=1, dtb=board_yaml, kernel=firmware_path, extra_args=clock_args_n1)
    simulation.add_node(node_id=2, dtb=board_yaml, kernel=firmware_path, extra_args=clock_args_n2)

    # Event-driven coordinator: asyncio.Event per node, set from Zenoh callback thread
    # via call_soon_threadsafe to avoid cross-thread asyncio state mutation.
    loop = asyncio.get_running_loop()
    zenoh_session = simulation._session
    done_events: dict[int, asyncio.Event] = {1: asyncio.Event(), 2: asyncio.Event()}
    uart_backlog: list[tuple[int, bytes]] = []

    def on_done(sample: zenoh.Sample) -> None:
        nid = int(str(sample.key_expr).split("/")[2])
        loop.call_soon_threadsafe(done_events[nid].set)

    def on_uart_tx(sample: zenoh.Sample) -> None:
        nid = int(str(sample.key_expr).split("/")[2])
        # Append from Zenoh callback thread; list.append is GIL-atomic in CPython.
        uart_backlog.append((nid, bytes(sample.payload.to_bytes())))

    def declare_done() -> object:
        return zenoh_session.declare_subscriber(SimTopic.COORD_DONE_WILDCARD, on_done)

    def declare_uart() -> object:
        return zenoh_session.declare_subscriber(SimTopic.SIM_UART_TX_WILDCARD, on_uart_tx)

    done_sub = await asyncio.to_thread(declare_done)
    uart_sub = await asyncio.to_thread(declare_uart)

    async def coordinator_loop() -> None:
        """
        Minimal Python coordinator implementing the barrier protocol.

        For every quantum:
          1. Wait (event-driven) until all nodes have signalled 'done'.
          2. Sleep _COORDINATOR_DELIVERY_DELAY_S to simulate message delivery.
          3. Forward queued UART payloads.
          4. Publish 'start' to each node, releasing it for the next quantum.
        """
        quantum = 0
        while True:
            # Wait for both nodes to complete the current quantum.
            await asyncio.gather(
                asyncio.wait_for(done_events[1].wait(), timeout=_VTA_TIMEOUT_S),
                asyncio.wait_for(done_events[2].wait(), timeout=_VTA_TIMEOUT_S),
            )

            # Simulate coordinator message delivery latency (what tests).
            await asyncio.sleep(_COORDINATOR_DELIVERY_DELAY_S)  # SLEEP_EXCEPTION: simulate coordinator delivery latency

            # Deliver cross-node UART messages.
            for src_nid, payload in list(uart_backlog):
                dst_nid = 2 if src_nid == 1 else 1
                topic = SimTopic.sim_uart_rx(dst_nid)

                def put_uart(t: str = topic, p: bytes = payload) -> None:
                    zenoh_session.put(t, p)

                await asyncio.to_thread(put_uart)
            uart_backlog.clear()

            # Reset events before sending start signals to avoid a race where
            # the next quantum's 'done' arrives before we clear.
            done_events[1].clear()
            done_events[2].clear()

            quantum += 1
            start_payload = quantum.to_bytes(8, "little")

            def put_start_1(p: bytes = start_payload) -> None:
                zenoh_session.put(SimTopic.clock_start(1), p)

            def put_start_2(p: bytes = start_payload) -> None:
                zenoh_session.put(SimTopic.clock_start(2), p)

            await asyncio.to_thread(put_start_1)
            await asyncio.to_thread(put_start_2)

    coord_task = asyncio.create_task(coordinator_loop())

    try:
        async with simulation as sim:
            vta = sim.vta
            assert vta is not None
            # Single quantum step: both nodes execute 1 ms of virtual time.
            # The coordinator introduces _COORDINATOR_DELIVERY_DELAY_S before releasing
            # nodes, so this step must take at least that long.
            t0 = time.monotonic()
            await vta.step(delta_ns=1_000_000, timeout=_VTA_TIMEOUT_S)
            t1 = time.monotonic()

            elapsed = t1 - t0
            assert elapsed >= _COORDINATOR_DELIVERY_DELAY_S, (
                f"VIOLATION: Clock advanced in {elapsed:.3f}s — "
                f"before coordinator completed {_COORDINATOR_DELIVERY_DELAY_S}s delivery delay!"
            )

    finally:
        coord_task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await coord_task
        await asyncio.to_thread(cast(Any, done_sub).undeclare)
        await asyncio.to_thread(cast(Any, uart_sub).undeclare)


@pytest.mark.asyncio
async def test_coordinator_fast_node_race(zenoh_router: str, zenoh_session: zenoh.Session) -> None:
    """
    / Postmortem: Proves that a node can immediately send 'done' the moment it
    receives 'start', without the coordinator dropping it due to a race condition with
    QuantumBarrier.reset().
    """
    from tools import vproto
    from tools.testing.virtmcu_test_suite.topics import SimTopic


    s = zenoh_session
    done_topic = SimTopic.coord_done(0)
    start_topic = SimTopic.clock_start(0)

    quanta_completed = 0
    max_quanta = 100

    start_event = asyncio.Event()
    loop = asyncio.get_running_loop()

    def on_start(sample: zenoh.Sample) -> None:
        nonlocal quanta_completed
        # INSTANTLY reply 'done' with the same quantum
        # If the coordinator reset race exists, it drops this because it hasn't reset yet!
        q = int.from_bytes(sample.payload.to_bytes(), "little")
        s.put(done_topic, vproto.CoordDoneReq(quantum=q, vtime_limit=0xFFFFFFFFFFFFFFFF, messages=[]).pack())
        quanta_completed += 1
        loop.call_soon_threadsafe(start_event.set)
    # Declare subscriber BEFORE entering the coordinator context — the framework's
    # routing barrier inside coordinator_subprocess covers it.
    sub = s.declare_subscriber(start_topic, on_start)

    try:
        async with coordinator_subprocess(
            binary=get_rust_binary_path(VirtmcuBinary.DETERMINISTIC_COORDINATOR),
            args=["--nodes", "1", "--connect", zenoh_router],
            zenoh_session=s,
        ):
            # Kickstart the coordinator
            s.put(SimTopic.coord_done(0), vproto.CoordDoneReq(quantum=1, vtime_limit=0xFFFFFFFFFFFFFFFF, messages=[]).pack())
            # Wait for 100 quanta to fly by. If a race exists, this hangs.
            async with asyncio.timeout(5.0):
                while quanta_completed < max_quanta:
                    await start_event.wait()
                    start_event.clear()
    except TimeoutError:
        pytest.fail(f"Coordinator stalled after {quanta_completed} quanta. Race condition likely triggered.")
    finally:
        typing.cast(typing.Any, sub).undeclare()

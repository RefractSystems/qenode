"""
SOTA Test Module: test_coordinator_sync

Context:
This module implements tests for the test_coordinator_sync subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coordinator_sync.
"""

import asyncio
import contextlib
import os
import subprocess
import time

import pytest
import zenoh

from tests.conftest import VirtualTimeAuthority
from tools.testing.utils import get_time_multiplier

_base_stall_timeout_ms = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
_STALL_TIMEOUT_MS = int(_base_stall_timeout_ms * get_time_multiplier())
# VTA timeout must exceed the QEMU stall timeout so QEMU can reply before Python gives up.
_VTA_TIMEOUT_S: float = max(30.0, _STALL_TIMEOUT_MS / 1000.0 + 10.0)

# Minimum wall-clock delay the coordinator must introduce before releasing nodes.
_COORDINATOR_DELIVERY_DELAY_S: float = 0.1


@pytest.mark.asyncio
async def test_coordinator_sync(zenoh_router, zenoh_session, qemu_launcher, tmp_path):
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

    board_yaml = tmp_path / "board.yaml"
    board_yaml.write_text(
        """
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
memory:
  - name: sram
    address: 0x40000000
    size: 0x1000000
peripherals:
  - name: uart0
    type: pl011
    address: 0x09000000
    interrupt: 4
"""
    )

    # Both nodes use slaved-icount mode.  -icount is mandatory for slaved-icount:
    # without it qemu_clock_get_ns() returns wall-clock time and quantum boundaries
    # never trigger correctly.  stall-timeout is intentionally omitted so that
    # VIRTMCU_STALL_TIMEOUT_MS is respected (CLAUDE.md §Clock §Stall-Timeout Contract).
    icount_args = ["-icount", "shift=0,align=off,sleep=off"]
    clock_args_n1 = [
        "-device",
        f"virtmcu-clock,node=1,mode=slaved-icount,router={zenoh_router},coordinated=true",
        "-chardev",
        f"virtmcu,id=chr1,node=1,router={zenoh_router},topic=sim/uart",
        "-serial",
        "chardev:chr1",
    ]
    clock_args_n2 = [
        "-device",
        f"virtmcu-clock,node=2,mode=slaved-icount,router={zenoh_router},coordinated=true",
        "-chardev",
        f"virtmcu,id=chr2,node=2,router={zenoh_router},topic=sim/uart",
        "-serial",
        "chardev:chr2",
    ]

    n1 = await qemu_launcher(
        str(board_yaml),
        firmware_path,
        ignore_clock_check=True,
        extra_args=["-S", *icount_args, *clock_args_n1],
    )
    n2 = await qemu_launcher(
        str(board_yaml),
        firmware_path,
        ignore_clock_check=True,
        extra_args=["-S", *icount_args, *clock_args_n2],
    )

    vta = VirtualTimeAuthority(zenoh_session, node_ids=[1, 2])

    # Event-driven coordinator: asyncio.Event per node, set from Zenoh callback thread
    # via call_soon_threadsafe to avoid cross-thread asyncio state mutation.
    loop = asyncio.get_running_loop()
    done_events: dict[int, asyncio.Event] = {1: asyncio.Event(), 2: asyncio.Event()}
    uart_backlog: list[tuple[int, bytes]] = []

    def on_done(sample) -> None:
        nid = int(str(sample.key_expr).split("/")[2])
        loop.call_soon_threadsafe(done_events[nid].set)

    def on_uart_tx(sample) -> None:
        nid = int(str(sample.key_expr).split("/")[2])
        # Append from Zenoh callback thread; list.append is GIL-atomic in CPython.
        uart_backlog.append((nid, bytes(sample.payload.to_bytes())))

    def declare_done() -> zenoh.Subscriber:
        return zenoh_session.declare_subscriber("sim/coord/*/done", on_done)

    def declare_uart() -> zenoh.Subscriber:
        return zenoh_session.declare_subscriber("sim/uart/*/tx", on_uart_tx)

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
                topic = f"sim/uart/{dst_nid}/rx"

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
                zenoh_session.put("sim/clock/start/1", p)

            def put_start_2(p: bytes = start_payload) -> None:
                zenoh_session.put("sim/clock/start/2", p)

            await asyncio.to_thread(put_start_1)
            await asyncio.to_thread(put_start_2)

    from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation

    sim = VirtmcuSimulation([n1, n2], vta)

    coord_task = asyncio.create_task(coordinator_loop())

    try:
        async with sim:
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
        await asyncio.to_thread(done_sub.undeclare)
        await asyncio.to_thread(uart_sub.undeclare)


@pytest.mark.asyncio
async def test_coordinator_fast_node_race(zenoh_router):
    """
    / Postmortem: Proves that a node can immediately send 'done' the moment it
    receives 'start', without the coordinator dropping it due to a race condition with
    QuantumBarrier.reset().
    """
    from tools.testing.virtmcu_test_suite.artifact_resolver import get_rust_binary_path
    cmd = [
        get_rust_binary_path("zenoh_coordinator"),
        "--pdes",
        "--nodes",
        "1",
        "--connect",
        zenoh_router,
    ]
    coord_task = asyncio.create_subprocess_exec(*cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    proc = await coord_task

    s = zenoh.open(zenoh.Config())
    # Give coordinator a moment to start and declare liveliness
    from tools.testing.virtmcu_test_suite.conftest_core import wait_for_zenoh_discovery

    await wait_for_zenoh_discovery(s, "sim/coordinator/liveliness")

    done_topic = "sim/coord/0/done"
    start_topic = "sim/clock/start/0"

    quanta_completed = 0
    max_quanta = 100

    start_event = asyncio.Event()
    loop = asyncio.get_running_loop()

    def on_start(sample):
        nonlocal quanta_completed
        # INSTANTLY reply 'done' with the same quantum
        # If the coordinator reset race exists, it drops this because it hasn't reset yet!
        s.put(done_topic, sample.payload.to_bytes())
        quanta_completed += 1
        loop.call_soon_threadsafe(start_event.set)

    sub = s.declare_subscriber(start_topic, on_start)

    # Kickstart the coordinator
    s.put("sim/coord/0/done", (1).to_bytes(8, "little"))

    try:
        # Wait for 100 quanta to fly by. If race condition exists, this will hang infinitely.
        async with asyncio.timeout(5.0):
            while quanta_completed < max_quanta:
                await start_event.wait()
                start_event.clear()
    except TimeoutError:
        proc.terminate()
        await proc.wait()
        pytest.fail(f"Coordinator stalled after {quanta_completed} quanta. Race condition likely triggered.")

    proc.terminate()
    await proc.wait()
    sub.undeclare()
    s.close()

# ZENOH_HACK_EXCEPTION: cyber_bridge mock requires declare_queryable which is not in SimulationTransport
"""
SOTA Test Module: test_cyber_bridge_stress

Context:
This module implements tests for the test_cyber_bridge_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_cyber_bridge_stress.
"""

from __future__ import annotations

import asyncio
import hashlib
import logging
import multiprocessing
import os
import time
import typing
from collections.abc import AsyncGenerator
from contextlib import AsyncExitStack
from pathlib import Path
from typing import TYPE_CHECKING, Any, cast

import pytest
import pytest_asyncio
import zenoh

from tools import vproto
from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary
from tools.testing.virtmcu_test_suite.constants import VirtmcuBinary
from tools.testing.virtmcu_test_suite.process import AsyncManagedProcess
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from pathlib import Path


from tools.testing.env import WORKSPACE_DIR

logger = logging.getLogger(__name__)

# Paths
BUILD_DIR = WORKSPACE_DIR / "target/release"

try:
    REPLAY_BIN = resolve_rust_binary(VirtmcuBinary.RESD_REPLAY)
except FileNotFoundError:
    # Allow test collection to proceed if binary is missing, test will fail later
    REPLAY_BIN = Path(WORKSPACE_DIR) / "target/release/resd_replay"
logger.info(f"DEBUG: REPLAY_BIN = {REPLAY_BIN}")


def create_resd(filename: str | Path, duration_ms: int) -> None:
    with Path(filename).open("wb") as f:
        f.write(b"RESD")
        f.write((1).to_bytes(1, "little"))
        f.write(b"\x00\x00\x00")

        # Block: ACCELERATION
        f.write((0x01).to_bytes(1, "little") + (0x0002).to_bytes(2, "little") + (0).to_bytes(2, "little"))
        # data_size: start_time(8) + metadata_size(8) + N samples
        num_samples = duration_ms
        f.write((8 + 8 + num_samples * 20).to_bytes(8, "little"))
        f.write((0).to_bytes(8, "little"))  # start_time
        f.write((0).to_bytes(8, "little"))  # metadata_size

        for i in range(num_samples):
            f.write(
                (i * 1_000_000).to_bytes(8, "little")
                + i.to_bytes(4, "little", signed=True)
                + (i * 2).to_bytes(4, "little", signed=True)
                + (i * 3).to_bytes(4, "little", signed=True)
            )


@pytest.mark.asyncio
async def test_multi_node_stress(zenoh_router: str, tmp_path: Path) -> None:
    ctx = multiprocessing.get_context("spawn")
    manager = ctx.Manager()

    num_nodes = 5
    duration_ms = 100
    tmp_dir = str(tmp_path)

    resd_files = []
    for i in range(num_nodes):
        f = Path(tmp_dir) / f"node_{i}.resd"
        create_resd(f, duration_ms)
        resd_files.append(f)

    # Use unique topic for parallel isolation
    unique_id = hashlib.sha256(tmp_path.name.encode()).hexdigest()[:8]
    unique_prefix = SimTopic.clock_unique_prefix(unique_id)

    # Start Zenoh session for mock QEMU
    from tools.testing.virtmcu_test_suite.conftest_core import open_client_session

    locator = zenoh_router
    session = open_client_session(connect=locator)

    node_vtimes = manager.dict(dict.fromkeys(range(num_nodes), 0))

    def on_query(query: zenoh.Query) -> None:
        # topic: sim/clock/advance/{id}
        logger.info(f"DEBUG: Received query on {query.key_expr}")
        try:
            node_id = int(str(query.key_expr).split("/")[-1])
            payload = cast(Any, query.payload).to_bytes()
            req = vproto.ClockAdvanceReq.unpack(payload)
            delta_ns, _mujoco_time, qn = req.delta_ns, req.mujoco_time_ns, req.quantum_number
            logger.info(f"DEBUG: Node {node_id} advance: delta={delta_ns}, qn={qn}")

            # Atomically update vtime
            node_vtimes[node_id] += delta_ns

            # Reply with ClockReadyPayload { current_vtime_ns, n_frames, error_code, quantum_number }
            reply_payload = vproto.ClockReadyResp(node_vtimes[node_id], 1, 0, qn).pack()
            query.reply(query.key_expr, reply_payload)
        except Exception as e:  # noqa: BLE001
            # In a stress test, we log and continue to keep the simulation session alive
            logger.error(f"DEBUG ERROR in on_query: {e}")

    # Subscribe to clock advance for all nodes
    queryables = []
    for i in range(num_nodes):
        q = session.declare_queryable(f"{unique_prefix}/advance/{i}", on_query)
        queryables.append(q)

    # Start resd_replay processes
    procs = []
    env = os.environ.copy()
    # Use the new robust connector env var
    env["ZENOH_CONNECT"] = f'["{locator}"]'
    env["ZENOH_TOPIC_PREFIX"] = unique_prefix

    async with AsyncExitStack() as stack:
        for i in range(num_nodes):
            p = await stack.enter_async_context(
                AsyncManagedProcess(
                    REPLAY_BIN,
                    resd_files[i],
                    str(i),
                    "1000000",
                    env=env,
                )
            )
            procs.append(p)

        # Wait for completion or timeout
        try:
            from tools.testing.utils import get_time_multiplier

            await asyncio.wait_for(asyncio.gather(*(p.wait() for p in procs)), timeout=30.0 * get_time_multiplier())
        except TimeoutError:
            logger.error("DEBUG: Stress test timed out!")
            pytest.fail("Timeout in multi-node stress test")

        # Verify exit codes and print logs
        for i, p in enumerate(procs):
            logger.info(f"DEBUG: Node {i} STDOUT: {p.stdout_text}")
            logger.info(f"DEBUG: Node {i} STDERR: {p.stderr_text}")
            if p.returncode != 0:
                logger.error(f"Node {i} failed with code {p.returncode}")
            assert p.returncode == 0, f"Node {i} failed"
            assert node_vtimes[i] >= (duration_ms - 1) * 1_000_000

    typing.cast(typing.Any, session).close()
    logger.info("Multi-node stress test PASSED")


@pytest_asyncio.fixture
async def mujoco_bridge_process() -> AsyncGenerator[tuple[AsyncManagedProcess, Path, int, int]]:
    """Fixture to manage the mujoco_bridge process lifecycle."""
    node_id = 42 + (os.getpid() % 1000)
    nu = 4
    nsensordata = 8
    bridge_bin = resolve_rust_binary(VirtmcuBinary.MUJOCO_BRIDGE)
    shm_path = Path("/") / "dev" / "shm" / f"virtmcu_mujoco_{node_id}"

    # Ensure no stale SHM exists
    if shm_path.exists():
        shm_path.unlink()

    async with AsyncManagedProcess(str(bridge_bin), str(node_id), str(nu), str(nsensordata)) as p:
        # Deterministically wait for the shared memory file to be created
        start_time = time.time()
        while not shm_path.exists() and time.time() - start_time < 5.0:
            if p.returncode is not None:
                break
            await asyncio.sleep(0.05)  # SLEEP_EXCEPTION: polling for shm creation

        yield p, shm_path, nu, nsensordata

    # Cleanup
    if shm_path.exists():
        shm_path.unlink()


@pytest.mark.asyncio
async def test_mujoco_bridge_shm(mujoco_bridge_process: tuple[asyncio.subprocess.Process, Path, int, int]) -> None:
    """
    Test the basic SHM lifecycle of the mujoco_bridge.
    """
    _p, shm_path, nu, nsensordata = mujoco_bridge_process

    # Check if shm segment exists
    assert shm_path.exists(), f"Shared memory {shm_path} was not created"

    # Verify size: Header(16) + (nu + nsensordata) * 8
    expected_size = 16 + (nu + nsensordata) * 8
    assert shm_path.stat().st_size == expected_size

    logger.info("MuJoCo Bridge SHM test PASSED")

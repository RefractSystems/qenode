"""
SOTA Test Module: test_cyber_bridge_stress

Context:
This module implements tests for the test_cyber_bridge_stress subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_cyber_bridge_stress.
"""

import asyncio
import contextlib
import logging
import multiprocessing
import os
import subprocess
import time
from pathlib import Path

import pytest
import vproto
import zenoh

from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary

logger = logging.getLogger(__name__)

# Paths
WORKSPACE_DIR = Path(Path(Path(__file__).resolve().parent) / "..")
BUILD_DIR = Path(WORKSPACE_DIR) / "target/release"

try:
    REPLAY_BIN = resolve_rust_binary("resd_replay")
except FileNotFoundError:
    # Allow test collection to proceed if binary is missing, test will fail later
    REPLAY_BIN = Path(WORKSPACE_DIR) / "target/release/resd_replay"
logger.info(f"DEBUG: REPLAY_BIN = {REPLAY_BIN}")


def create_resd(filename, duration_ms):
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
async def test_multi_node_stress(zenoh_router, tmp_path):
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
    import uuid

    unique_prefix = f"sim/clock/{uuid.uuid4().hex[:8]}"

    # Start Zenoh session for mock QEMU
    conf = zenoh.Config()
    # Force a local locator to ensure connectivity
    locator = zenoh_router
    conf.insert_json5("connect/endpoints", f'["{locator}"]')
    session = zenoh.open(conf)

    node_vtimes = manager.dict(dict.fromkeys(range(num_nodes), 0))

    def on_query(query):
        # topic: sim/clock/advance/{id}
        logger.info(f"DEBUG: Received query on {query.key_expr}")
        try:
            node_id = int(str(query.key_expr).split("/")[-1])
            payload = query.payload.to_bytes()
            req = vproto.ClockAdvanceReq.unpack(payload)
            delta_ns, _mujoco_time, qn = req.delta_ns, req.mujoco_time_ns, req.quantum_number
            logger.info(f"DEBUG: Node {node_id} advance: delta={delta_ns}, qn={qn}")

            # Atomically update vtime
            node_vtimes[node_id] += delta_ns

            # Reply with ClockReadyPayload { current_vtime_ns, n_frames, error_code, quantum_number }
            reply_payload = vproto.ClockReadyResp(node_vtimes[node_id], 1, 0, qn).pack()
            query.reply(query.key_expr, reply_payload)
        except Exception as e:
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

    for i in range(num_nodes):
        p = await asyncio.create_subprocess_exec(
            REPLAY_BIN,
            resd_files[i],
            str(i),
            "1000000",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )
        procs.append(p)

    # Wait for completion or timeout
    try:
        from tools.testing.utils import get_time_multiplier

        await asyncio.wait_for(asyncio.gather(*(p.wait() for p in procs)), timeout=30.0 * get_time_multiplier())
    except TimeoutError:
        logger.error("DEBUG: Stress test timed out!")
        for p in procs:
            with contextlib.suppress(Exception):
                p.kill()
        pytest.fail("Timeout in multi-node stress test")

    # Verify exit codes and print logs
    for i, p in enumerate(procs):
        stdout, stderr = await p.communicate()
        logger.info(f"DEBUG: Node {i} STDOUT: {stdout.decode()}")
        logger.info(f"DEBUG: Node {i} STDERR: {stderr.decode()}")
        if p.returncode != 0:
            logger.error(f"Node {i} failed with code {p.returncode}")
        assert p.returncode == 0, f"Node {i} failed"
        assert node_vtimes[i] >= (duration_ms - 1) * 1_000_000

    session.close()
    logger.info("Multi-node stress test PASSED")


@pytest.mark.asyncio
async def test_mujoco_bridge_shm(zenoh_router):  # noqa: ARG001
    # Test mujoco_bridge shared memory creation and layout

    # Use worker_id logic if needed, but since it's a simple test, a fixed ID like 42 is fine
    # as long as we clean it up and tests are isolated. Or derive it from the test name/PID.
    # To be completely safe in parallel without random, we can use the current process PID.
    import os

    node_id = 42 + (os.getpid() % 1000)
    nu = 4
    nsensordata = 8

    bridge_bin = resolve_rust_binary("mujoco_bridge")

    # Run bridge briefly
    p = subprocess.Popen(
        [str(bridge_bin), str(node_id), str(nu), str(nsensordata)], stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )

    time.sleep(1.0)
    p.kill()
    _stdout, _stderr = p.communicate()

    # Check if shm segment exists
    shm_path = f"/dev/shm/virtmcu_mujoco_{node_id}"
    assert Path(shm_path).exists()

    # Verify size: Header(16) + (4+8)*8 = 16 + 96 = 112
    # Wait, size is Header + (nsensordata + nu) * 8
    expected_size = 16 + (nu + nsensordata) * 8
    assert Path(shm_path).stat().st_size == expected_size

    # Cleanup
    if Path(shm_path).exists():
        Path(shm_path).unlink()
    logger.info("MuJoCo Bridge SHM test PASSED")

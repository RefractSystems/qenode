import asyncio
import contextlib
import multiprocessing
import os
import struct
import subprocess
import time
from pathlib import Path

import pytest
import zenoh

# Paths
WORKSPACE_DIR = Path(Path(Path(__file__).resolve().parent) / "..")
BUILD_DIR = Path(WORKSPACE_DIR) / "target/release"
REPLAY_BIN = Path(BUILD_DIR) / "resd_replay"
if not REPLAY_BIN.exists():
    REPLAY_BIN = Path(WORKSPACE_DIR) / "tools/cyber_bridge/target/release/resd_replay"
print(f"DEBUG: REPLAY_BIN = {REPLAY_BIN}")


def create_resd(filename, duration_ms):
    with Path(filename).open("wb") as f:
        f.write(b"RESD")
        f.write(struct.pack("<B", 1))
        f.write(b"\x00\x00\x00")

        # Block: ACCELERATION
        f.write(struct.pack("<BHH", 0x01, 0x0002, 0))
        # data_size: start_time(8) + metadata_size(8) + N samples
        num_samples = duration_ms
        f.write(struct.pack("<Q", 8 + 8 + num_samples * 20))
        f.write(struct.pack("<Q", 0))  # start_time
        f.write(struct.pack("<Q", 0))  # metadata_size

        for i in range(num_samples):
            f.write(struct.pack("<Qiii", i * 1_000_000, i, i * 2, i * 3))


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
        print(f"DEBUG: Received query on {query.key_expr}")
        try:
            node_id = int(str(query.key_expr).split("/")[-1])
            payload = query.payload.to_bytes()
            delta_ns, _mujoco_time = struct.unpack("<QQ", payload)
            print(f"DEBUG: Node {node_id} advance: delta={delta_ns}")

            # Atomically update vtime
            node_vtimes[node_id] += delta_ns

            # Reply with ClockReadyPayload { current_vtime_ns, n_frames }
            reply_payload = struct.pack("<QII", node_vtimes[node_id], 1, 0)
            query.reply(query.key_expr, reply_payload)
        except Exception as e:
            print(f"DEBUG ERROR in on_query: {e}")

    # Subscribe to clock advance for all nodes
    queryables = []
    for i in range(num_nodes):
        q = session.declare_queryable(f"{unique_prefix}/advance/{i}", on_query)
        queryables.append(q)

    from tests.conftest import wait_for_zenoh_discovery
    for i in range(num_nodes):
        await wait_for_zenoh_discovery(session, f"{unique_prefix}/advance/{i}")

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
        await asyncio.wait_for(asyncio.gather(*(p.wait() for p in procs)), timeout=30.0)
    except TimeoutError:
        print("DEBUG: Stress test timed out!")
        for p in procs:
            with contextlib.suppress(Exception):
                p.kill()
        pytest.fail("Timeout in multi-node stress test")

    # Verify exit codes and print logs
    for i, p in enumerate(procs):
        stdout, stderr = await p.communicate()
        print(f"DEBUG: Node {i} STDOUT: {stdout.decode()}")
        print(f"DEBUG: Node {i} STDERR: {stderr.decode()}")
        if p.returncode != 0:
            print(f"Node {i} failed with code {p.returncode}")
        assert p.returncode == 0, f"Node {i} failed"
        assert node_vtimes[i] >= (duration_ms - 1) * 1_000_000

    session.close()
    print("Multi-node stress test PASSED")


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

    bridge_bin = Path(BUILD_DIR) / "mujoco_bridge"
    if not bridge_bin.exists():
        bridge_bin = Path(WORKSPACE_DIR) / "tools/cyber_bridge/target/release/mujoco_bridge"

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
    print("MuJoCo Bridge SHM test PASSED")

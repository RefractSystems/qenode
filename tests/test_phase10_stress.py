import asyncio
import multiprocessing
import os
import struct
import subprocess
import time

import pytest
import zenoh

# Paths
WORKSPACE_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
# Use build_cov if it exists, otherwise build
BUILD_DIR = os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/build_cov")
if not os.path.exists(BUILD_DIR):
    BUILD_DIR = os.path.join(WORKSPACE_DIR, "tools/cyber_bridge/build")
REPLAY_BIN = os.path.join(BUILD_DIR, "resd_replay")


def create_resd(filename, duration_ms):
    with open(filename, "wb") as f:
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
async def test_multi_node_stress():
    manager = multiprocessing.Manager()

    num_nodes = 5
    duration_ms = 100
    tmp_dir = "/tmp/virtmcu_stress_phase10"
    os.makedirs(tmp_dir, exist_ok=True)

    resd_files = []
    for i in range(num_nodes):
        f = os.path.join(tmp_dir, f"node_{i}.resd")
        create_resd(f, duration_ms)
        resd_files.append(f)

    # Start Zenoh session for mock QEMU
    conf = zenoh.Config()
    # Force a local locator to ensure connectivity
    locator = "tcp/127.0.0.1:7447"
    conf.insert_json5("listen/endpoints", f'["{locator}"]')
    session = zenoh.open(conf)

    node_vtimes = manager.dict({i: 0 for i in range(num_nodes)})

    def on_query(query):
        # topic: sim/clock/advance/{id}
        try:
            node_id = int(str(query.key_expr).split("/")[-1])
            payload = query.payload.to_bytes()
            delta_ns, mujoco_time = struct.unpack("<QQ", payload)

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
        q = session.declare_queryable(f"sim/clock/advance/{i}", on_query)
        queryables.append(q)

    # Give Zenoh time to propagate queryables
    await asyncio.sleep(2.0)

    # Start resd_replay processes
    procs = []
    env = os.environ.copy()
    # Use the new robust connector env var
    env["ZENOH_CONNECT"] = f'["{locator}"]'

    for i in range(num_nodes):
        p = await asyncio.create_subprocess_exec(
            REPLAY_BIN, resd_files[i], str(i), "1000000",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env
        )
        procs.append(p)

    # Wait for completion or timeout
    try:
        await asyncio.wait_for(asyncio.gather(*(p.wait() for p in procs)), timeout=30.0)
    except asyncio.TimeoutError:
        print("DEBUG: Stress test timed out!")
        for p in procs:
            try:
                p.kill()
            except Exception:
                pass
        pytest.fail("Timeout in multi-node stress test")

    # Verify exit codes and print logs
    for i, p in enumerate(procs):
        stdout, stderr = await p.communicate()
        if p.returncode != 0:
            print(f"Node {i} failed with code {p.returncode}")
            print(f"STDOUT: {stdout.decode()}")
            print(f"STDERR: {stderr.decode()}")
        assert p.returncode == 0, f"Node {i} failed"
        assert node_vtimes[i] >= (duration_ms - 1) * 1_000_000

    session.close()
    print("Multi-node stress test PASSED")

@pytest.mark.asyncio
async def test_mujoco_bridge_shm():
    # Test mujoco_bridge shared memory creation and layout
    node_id = 42
    nu = 4
    nsensordata = 8

    bridge_bin = os.path.join(BUILD_DIR, "mujoco_bridge")

    # Run bridge briefly
    p = subprocess.Popen([bridge_bin, str(node_id), str(nu), str(nsensordata)],
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    time.sleep(1.0)
    p.kill()
    stdout, stderr = p.communicate()

    # Check if shm segment exists
    shm_path = f"/dev/shm/virtmcu_mujoco_{node_id}"
    assert os.path.exists(shm_path)

    # Verify size: Header(16) + (4+8)*8 = 16 + 96 = 112
    # Wait, size is Header + (nsensordata + nu) * 8
    expected_size = 16 + (nu + nsensordata) * 8
    assert os.path.getsize(shm_path) == expected_size

    # Cleanup
    if os.path.exists(shm_path):
        os.remove(shm_path)
    print("MuJoCo Bridge SHM test PASSED")

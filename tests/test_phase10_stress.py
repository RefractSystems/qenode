import os
import struct
import subprocess
import time

import pytest
import zenoh

# Paths
WORKSPACE_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
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
    session = zenoh.open(conf)

    node_vtimes = {i: 0 for i in range(num_nodes)}

    def on_query(query):
        # topic: sim/clock/advance/{id}
        node_id = int(str(query.key_expr).split("/")[-1])
        payload = query.payload
        delta_ns, mujoco_time = struct.unpack("<QQ", payload)

        node_vtimes[node_id] += delta_ns

        # Reply with ClockReadyPayload { current_vtime_ns, n_frames }
        reply_payload = struct.pack("<QI", node_vtimes[node_id], 1)
        query.reply(zenoh.Sample(query.key_expr, reply_payload))

    # Subscribe to clock advance for all nodes
    queryables = []
    for i in range(num_nodes):
        q = session.declare_queryable(f"sim/clock/advance/{i}", on_query)
        queryables.append(q)

    # Start resd_replay processes
    procs = []
    for i in range(num_nodes):
        p = subprocess.Popen(
            [REPLAY_BIN, resd_files[i], str(i), "1000000"], stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
        procs.append(p)

    # Wait for completion or timeout
    start_t = time.time()
    while any(p.poll() is None for p in procs):
        if time.time() - start_t > 30:
            for p in procs:
                p.kill()
            pytest.fail("Timeout in multi-node stress test")
        time.sleep(0.1)

    # Verify exit codes
    for i, p in enumerate(procs):
        stdout, stderr = p.communicate()
        assert p.returncode == 0, f"Node {i} failed: {stderr.decode()}"
        assert node_vtimes[i] >= (duration_ms - 1) * 1_000_000

    session.close()
    print("Multi-node stress test PASSED")

import asyncio
import struct
from pathlib import Path

import pytest

from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary


@pytest.mark.asyncio
async def test_det8_pcap_determinism(zenoh_router, zenoh_session, tmp_path):
    coordinator_bin = resolve_rust_binary("deterministic_coordinator")

    world_yaml = tmp_path / "world.yaml"
    world_yaml.write_text("""
nodes:
  - id: 0
  - id: 1
topology:
  global_seed: 42
  links:
    - type: uart
      nodes: [0, 1]
    """)

    async def run_simulation(pcap_path: Path):
        proc = await asyncio.create_subprocess_exec(
            "stdbuf", "-oL",
            str(coordinator_bin),
            "--connect", zenoh_router,
            "--topology", str(world_yaml),
            "--pcap-log", str(pcap_path),
            "--nodes", "2",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )

        try:
            await asyncio.sleep(0.5)  # SLEEP_EXCEPTION: pending ARCH-20
            msg_payload_eth = b"ETH"
            msg_eth = struct.pack("<IIIQQBI", 1, 0, 2, 1000, 1, 0, len(msg_payload_eth)) + msg_payload_eth
            msg_payload_uart1 = b"UART1"
            msg_uart1 = struct.pack("<IIIQQBI", 1, 0, 1, 2000, 2, 1, len(msg_payload_uart1)) + msg_payload_uart1
            msg_payload_uart2 = b"UART2"
            msg_uart2 = struct.pack("<IIIQQBI", 1, 1, 0, 3000, 3, 1, len(msg_payload_uart2)) + msg_payload_uart2

            def _send():
                zenoh_session.put("sim/coord/0/tx", msg_eth)
                zenoh_session.put("sim/coord/0/tx", msg_uart1)
                zenoh_session.put("sim/coord/1/tx", msg_uart2)
                import time
                time.sleep(0.2) # SLEEP_EXCEPTION: mock node simulating execution time to avoid Zenoh publisher race condition
                zenoh_session.put("sim/coord/0/done", struct.pack("<Q", 1))
                zenoh_session.put("sim/coord/1/done", struct.pack("<Q", 1))

            await asyncio.to_thread(_send)
            await asyncio.sleep(0.5)  # SLEEP_EXCEPTION: pending ARCH-20

        finally:
            try:
                proc.terminate()
                await proc.wait()
            except Exception:
                pass

            assert proc.stderr is not None
            stderr = (await proc.stderr.read()).decode()
            assert proc.stdout is not None
            stdout = (await proc.stdout.read()).decode()
            print("STDOUT:", stdout)
            print("STDERR:", stderr)
            if proc.returncode != 0 and proc.returncode != -15:
                print(f"Coordinator exited with code {proc.returncode}")

    pcap1 = tmp_path / "run1.pcap"
    await run_simulation(pcap1)

    pcap2 = tmp_path / "run2.pcap"
    await run_simulation(pcap2)

    assert pcap1.exists()
    assert pcap2.exists()

    with pcap1.open("rb") as f1, pcap2.open("rb") as f2:
        content1 = f1.read()
        content2 = f2.read()

    assert content1 == content2, "PCAP files are not bit-identical!"
    assert len(content1) > 24, "PCAP file only contains the global header, no packets written!"


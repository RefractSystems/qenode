"""
SOTA Test Module: test_pcap_determinism

Context:
This module implements tests for the test_pcap_determinism subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_pcap_determinism.
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
from pathlib import Path
from typing import TYPE_CHECKING, Any, cast

import pytest

from tools.testing.virtmcu_test_suite.artifact_resolver import resolve_rust_binary
from tools.testing.virtmcu_test_suite.conftest_core import wait_for_zenoh_discovery

if TYPE_CHECKING:
    from pathlib import Path

    import zenoh


logger = logging.getLogger(__name__)


@pytest.mark.asyncio
async def test_pcap_determinism(zenoh_router: str, zenoh_session: zenoh.Session, tmp_path: Path) -> None:
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

    async def run_simulation(pcap_path: Path) -> None:
        proc = await asyncio.create_subprocess_exec(
            "stdbuf",
            "-oL",
            str(coordinator_bin),
            "--connect",
            zenoh_router,
            "--topology",
            str(world_yaml),
            "--pcap-log",
            str(pcap_path),
            "--nodes",
            "2",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )

        try:
            await wait_for_zenoh_discovery(zenoh_session, "sim/coord/alive")

            msg_payload_eth = b"ETH"
            msg_eth = (
                (1).to_bytes(4, "little")
                + (0).to_bytes(4, "little")
                + (2).to_bytes(4, "little")
                + (1000).to_bytes(8, "little")
                + (1).to_bytes(8, "little")
                + (0).to_bytes(1, "little")
                + len(msg_payload_eth).to_bytes(4, "little")
                + msg_payload_eth
            )
            msg_payload_uart1 = b"UART1"
            msg_uart1 = (
                (1).to_bytes(4, "little")
                + (0).to_bytes(4, "little")
                + (1).to_bytes(4, "little")
                + (2000).to_bytes(8, "little")
                + (2).to_bytes(8, "little")
                + (1).to_bytes(1, "little")
                + len(msg_payload_uart1).to_bytes(4, "little")
                + msg_payload_uart1
            )
            msg_payload_uart2 = b"UART2"
            msg_uart2 = (
                (1).to_bytes(4, "little")
                + (1).to_bytes(4, "little")
                + (0).to_bytes(4, "little")
                + (3000).to_bytes(8, "little")
                + (3).to_bytes(8, "little")
                + (1).to_bytes(1, "little")
                + len(msg_payload_uart2).to_bytes(4, "little")
                + msg_payload_uart2
            )

            loop = asyncio.get_running_loop()
            quantum_event = asyncio.Event()

            def on_start(sample: object) -> None:
                q = int.from_bytes(cast(Any, sample).payload.to_bytes(), "little")
                if q == 2:
                    loop.call_soon_threadsafe(quantum_event.set)

            sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber("sim/clock/start/0", on_start))

            try:

                def _send() -> None:
                    zenoh_session.put("sim/coord/0/tx", msg_eth)
                    zenoh_session.put("sim/coord/0/tx", msg_uart1)
                    zenoh_session.put("sim/coord/1/tx", msg_uart2)

                    zenoh_session.put("sim/coord/0/done", (1).to_bytes(8, "little"))
                    zenoh_session.put("sim/coord/1/done", (1).to_bytes(8, "little"))

                await asyncio.to_thread(_send)
                await asyncio.wait_for(quantum_event.wait(), timeout=5.0)
            finally:
                await asyncio.to_thread(sub.undeclare)

        finally:
            with contextlib.suppress(Exception):
                proc.terminate()
                await proc.wait()

            assert proc.stderr is not None
            stderr = (await proc.stderr.read()).decode()
            assert proc.stdout is not None
            stdout = (await proc.stdout.read()).decode()
            logger.info("STDOUT: %s", stdout)
            logger.info("STDERR: %s", stderr)
            if proc.returncode != 0 and proc.returncode != -15:
                logger.info(f"Coordinator exited with code {proc.returncode}")

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

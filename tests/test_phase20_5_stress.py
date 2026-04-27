import asyncio
import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_spi_stress_baremetal(qemu_launcher, zenoh_session, zenoh_router, tmp_path):
    """
    Stress test for Phase 20.5: Perform 10,000 rapid SPI transactions
    through the Zenoh SPI bridge to verify backpressure, lock safety,
    and throughput stability.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    yaml_path = Path(workspace_root) / "test/phase20_5/spi_test.yaml"
    dtb_path = tmp_path / "spi_stress.dtb"
    kernel_path = Path(workspace_root) / "test/phase20_5/spi_stress.elf"

    router_endpoint = zenoh_router

    if not Path(kernel_path).exists():
        subprocess.run(["make", "-C", "test/phase20_5"], check=True, cwd=workspace_root)

    with Path(yaml_path).open() as f:
        config = f.read()

    config = config.replace(
        "- name: spi_echo\n    type: spi-echo",
        f"- name: spi_echo\n    type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}",
    )
    if f"router: {router_endpoint}" not in config:
        config = config.replace(
            "type: spi-echo", f"type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}"
        )

    temp_yaml = tmp_path / "spi_stress_zenoh.yaml"
    with Path(temp_yaml).open("w") as f:
        f.write(config)

    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", str(temp_yaml), "--out-dtb", str(dtb_path)], check=True, cwd=workspace_root
    )

    topic = "sim/spi/spi0/0"

    received_queries = 0

    def on_query(query):
        nonlocal received_queries
        received_queries += 1
        payload = query.payload
        if payload:
            data_bytes = payload.to_bytes()
            if len(data_bytes) >= 28:
                data = data_bytes[24:28]
                query.reply(query.key_expr, data)

    _ = await asyncio.to_thread(lambda: zenoh_session.declare_queryable(topic, on_query))

    bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=["-S"])
    await bridge.start_emulation()

    success = False
    for _ in range(200): # Allow up to 20 seconds for 10k messages
        if b"P" in bridge.uart_buffer.encode():
            success = True
            break
        if b"F" in bridge.uart_buffer.encode():
            pytest.fail(f"Firmware signaled SPI stress test FAILURE. UART: {bridge.uart_buffer}")
        await asyncio.sleep(0.1)

    assert success, f"Firmware timed out. Received {received_queries}/10000 queries. UART: {bridge.uart_buffer!r}"
    assert received_queries == 10000, f"Expected exactly 10,000 SPI transactions, got {received_queries}"


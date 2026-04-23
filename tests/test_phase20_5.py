import asyncio
import subprocess
from pathlib import Path

import pytest


@pytest.mark.asyncio
async def test_spi_echo_baremetal(qemu_launcher, zenoh_session, zenoh_router, tmp_path):
    """
    Task 20.5.4: SPI Loopback/Echo Firmware.
    Verify that the ARM bare-metal firmware can perform full-duplex SPI
    transactions against a Zenoh-backed SPI bridge.
    """
    workspace_root = Path(Path(Path(__file__).parent.resolve().parent))
    yaml_path = Path(workspace_root) / "test/phase20_5/spi_test.yaml"
    dtb_path = tmp_path / "spi_test.dtb"
    kernel_path = Path(workspace_root) / "test/phase20_5/spi_echo.elf"

    # Get the actual router endpoint from the fixture
    router_endpoint = zenoh_router

    # 1. Build firmware if missing
    if not Path(kernel_path).exists():
        subprocess.run(["make", "-C", "test/phase20_5"], check=True, cwd=workspace_root)

    # 2. Generate DTB using yaml2qemu
    # Create a temporary yaml with Zenoh SPI Bridge
    with Path(yaml_path).open() as f:
        config = f.read()

    # Replace spi-echo with SPI.ZenohBridge and add router property
    # Target specifically the spi_echo device
    config = config.replace(
        "- name: spi_echo\n    type: spi-echo",
        f"- name: spi_echo\n    type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}",
    )
    if f"router: {router_endpoint}" not in config:
        # Fallback
        config = config.replace(
            "type: spi-echo", f"type: SPI.ZenohBridge\n    properties:\n      router: {router_endpoint}"
        )

    temp_yaml = tmp_path / "spi_test_zenoh.yaml"
    with Path(temp_yaml).open("w") as f:
        f.write(config)

    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", str(temp_yaml), "--out-dtb", str(dtb_path)], check=True, cwd=workspace_root
    )

    # 3. Setup Zenoh Echo
    # Topic: sim/spi/{id}/{cs} -> default id is 'spi0', cs is 0
    topic = "sim/spi/spi0/0"

    def on_query(query):
        payload = query.payload
        if payload:
            data_bytes = payload.to_bytes()
            if len(data_bytes) >= 16 + 4:
                # Header is 16 bytes, data is 4 bytes
                data = data_bytes[16:20]
                # Echo back
                query.reply(query.key_expr, data)

    _ = await asyncio.to_thread(lambda: zenoh_session.declare_queryable(topic, on_query))

    # 4. Launch QEMU
    bridge = await qemu_launcher(dtb_path, kernel_path, extra_args=["-S"])
    await bridge.start_emulation()

    # 4. Wait for firmware to complete.
    # spi_echo.S writes 'P' (success) or 'F' (failure) to UART0.
    success = False
    for _ in range(100):
        # Check UART output
        if b"P" in bridge.uart_buffer.encode():
            success = True
            break
        if b"F" in bridge.uart_buffer.encode():
            pytest.fail(f"Firmware signaled SPI verification FAILURE. UART: {bridge.uart_buffer}")
        await asyncio.sleep(0.1)

    if not success:
        print(f"DEBUG: UART Buffer: {bridge.uart_buffer!r}")

    assert success, f"Firmware timed out without signaling success (P). UART: {bridge.uart_buffer!r}"

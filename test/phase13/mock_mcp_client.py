import asyncio
import json
import os
import sys

# Add workspace to path to import node_manager
WORKSPACE_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.append(WORKSPACE_DIR)

async def main():
    print("Connecting to MCP server...")
    # Launch the MCP server as a subprocess
    proc = await asyncio.create_subprocess_exec(
        sys.executable, "-m", "tools.mcp_server",
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        cwd=WORKSPACE_DIR
    )

    async def send_json(obj):
        data = json.dumps(obj) + "\n"
        proc.stdin.write(data.encode())
        await proc.stdin.drain()

    async def recv_json():
        line = await proc.stdout.readline()
        if not line:
            return None
        return json.loads(line.decode())

    # 1. Initialize
    await send_json({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "mock-client", "version": "1.0.0"}
        }
    })
    res = await recv_json()
    assert "result" in res, f"Initialization failed: {res}"

    # 2. Provision Board
    board_config = """
machine:
  name: test-node
  cpu: cortex-a15
  ram: 128M
peripherals:
  - type: pl011
    address: 0x09000000
    irq: 1
"""
    await send_json({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "provision_board",
            "arguments": {
                "node_id": "node0",
                "board_config": board_config,
                "config_type": "yaml"
            }
        }
    })
    res = await recv_json()
    print(f"Provision result: {res['result']['content'][0]['text']}")

    # 3. Flash Firmware
    firmware_path = os.path.join(WORKSPACE_DIR, "test", "phase1", "hello.elf")
    await send_json({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "flash_firmware",
            "arguments": {
                "node_id": "node0",
                "firmware_path": firmware_path
            }
        }
    })
    res = await recv_json()
    print(f"Flash result: {res['result']['content'][0]['text']}")

    # 4. Start Node
    await send_json({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "start_node",
            "arguments": {"node_id": "node0"}
        }
    })
    res = await recv_json()
    print(f"Start result: {res['result']['content'][0]['text']}")

    # Wait for QEMU to boot a bit
    await asyncio.sleep(2)

    # 5. Read CPU State
    await send_json({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "read_cpu_state",
            "arguments": {"node_id": "node0"}
        }
    })
    res = await recv_json()
    print(f"CPU State:\n{res['result']['content'][0]['text']}")

    # 6. Stop Node
    await send_json({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "stop_node",
            "arguments": {"node_id": "node0"}
        }
    })
    res = await recv_json()
    print(f"Stop result: {res['result']['content'][0]['text']}")

    # Cleanup
    proc.terminate()
    await proc.wait()

if __name__ == "__main__":
    asyncio.run(main())

import asyncio
import json
import os
import sys

WORKSPACE_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.append(WORKSPACE_DIR)

async def main():
    print("Connecting to MCP server...")
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
        # print(f"<- {line.decode().strip()}")
        return json.loads(line.decode())

    # Initialize
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
    await recv_json()
    await send_json({"jsonrpc": "2.0", "method": "notifications/initialized"})

    yaml_path = os.path.join(WORKSPACE_DIR, "test", "phase3", "test_board.yaml")
    with open(yaml_path, "r") as f:
        board_config = f.read()

    firmware_path = os.path.join(WORKSPACE_DIR, "test", "phase1", "hello.elf")

    # Provision and start 2 nodes
    for i in range(2):
        node_id = f"node{i}"
        print(f"Provisioning {node_id}...")
        await send_json({
            "jsonrpc": "2.0",
            "id": 10 + i,
            "method": "tools/call",
            "params": {
                "name": "provision_board",
                "arguments": {"node_id": node_id, "board_config": board_config}
            }
        })
        await recv_json()

        print(f"Flashing {node_id}...")
        await send_json({
            "jsonrpc": "2.0",
            "id": 20 + i,
            "method": "tools/call",
            "params": {
                "name": "flash_firmware",
                "arguments": {"node_id": node_id, "firmware_path": firmware_path}
            }
        })
        await recv_json()

        print(f"Starting {node_id}...")
        await send_json({
            "jsonrpc": "2.0",
            "id": 30 + i,
            "method": "tools/call",
            "params": {
                "name": "start_node",
                "arguments": {"node_id": node_id}
            }
        })
        await recv_json()

    print("Both nodes started. Waiting for them to boot...")
    await asyncio.sleep(3)

    # Check status resource
    print("Checking simulation status...")
    await send_json({
        "jsonrpc": "2.0",
        "id": 40,
        "method": "resources/read",
        "params": {"uri": "virtmcu://simulation/status"}
    })
    res = await recv_json()
    status = json.loads(res["result"]["contents"][0]["text"])
    assert len(status["nodes"]) == 2
    for n in status["nodes"]:
        assert n["status"] == "running"

    # Read PC from both
    for i in range(2):
        node_id = f"node{i}"
        print(f"Reading CPU state for {node_id}...")
        await send_json({
            "jsonrpc": "2.0",
            "id": 50 + i,
            "method": "tools/call",
            "params": {
                "name": "read_cpu_state",
                "arguments": {"node_id": node_id}
            }
        })
        res = await recv_json()
        assert "R0" in res["result"]["content"][0]["text"] or "PC" in res["result"]["content"][0]["text"]

    # Stop all
    for i in range(2):
        node_id = f"node{i}"
        print(f"Stopping {node_id}...")
        await send_json({
            "jsonrpc": "2.0",
            "id": 60 + i,
            "method": "tools/call",
            "params": {
                "name": "stop_node",
                "arguments": {"node_id": node_id}
            }
        })
        await recv_json()

    proc.terminate()
    await proc.wait()
    print("Multi-node MCP test passed!")

if __name__ == "__main__":
    asyncio.run(main())

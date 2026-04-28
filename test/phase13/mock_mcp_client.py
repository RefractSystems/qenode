import asyncio
import json
import sys
from pathlib import Path

WORKSPACE_DIR = Path(__file__).resolve().parent.parent.parent
sys.path.append(WORKSPACE_DIR)


async def main():
    print("Connecting to MCP server...")
    proc = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "tools.mcp_server",
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        cwd=WORKSPACE_DIR,
    )

    async def send_json(obj):
        data = json.dumps(obj) + "\n"
        proc.stdin.write(data.encode())
        await proc.stdin.drain()

    async def recv_json():
        line = await proc.stdout.readline()
        if not line:
            return None
        print(f"<- {line.decode().strip()}")
        return json.loads(line.decode())

    # 1. Initialize
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "mock-client", "version": "1.0.0"},
            },
        }
    )
    await recv_json()

    # Send initialized notification
    await send_json({"jsonrpc": "2.0", "method": "notifications/initialized"})

    # Read the exact YAML file used in phase 3.5 tests
    yaml_path = Path(WORKSPACE_DIR) / "test" / "phase3" / "test_board.yaml"
    with Path(yaml_path).open() as f:
        board_config = f.read()

    # 2. Provision Board
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "provision_board",
                "arguments": {"node_id": "node0", "board_config": board_config, "config_type": "yaml"},
            },
        }
    )
    await recv_json()

    # 3. Flash Firmware
    firmware_path = Path(WORKSPACE_DIR) / "test" / "phase1" / "hello.elf"
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "flash_firmware",
                "arguments": {"node_id": "node0", "firmware_path": str(firmware_path)},
            },
        }
    )
    await recv_json()

    # 4. Start Node
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "start_node", "arguments": {"node_id": "node0"}},
        }
    )
    await recv_json()

    await asyncio.sleep(2)  # SLEEP_EXCEPTION: deliberate yielding

    # 5. Read CPU State
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "read_cpu_state", "arguments": {"node_id": "node0"}},
        }
    )
    await recv_json()

    # 6. Stop Node
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "stop_node", "arguments": {"node_id": "node0"}},
        }
    )
    await recv_json()

    # We must explicitly tell the server to exit before waiting, otherwise it hangs listening to stdin
    # MCP doesn't have an explicit 'shutdown' in the python sdk that breaks the loop usually
    # Just terminate and wait.
    proc.terminate()
    try:
        await asyncio.wait_for(proc.wait(), timeout=5.0)
    except TimeoutError:
        proc.kill()
        await proc.wait()
    print("Done.")


if __name__ == "__main__":
    asyncio.run(main())

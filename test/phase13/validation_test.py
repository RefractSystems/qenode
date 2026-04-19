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

    async def log_stderr():
        while True:
            line = await proc.stderr.readline()
            if not line: break
            print(f"Server stderr: {line.decode().strip()}", file=sys.stderr)

    stderr_task = asyncio.create_task(log_stderr())

    async def send_json(obj):
        data = json.dumps(obj) + "\n"
        proc.stdin.write(data.encode())
        await proc.stdin.drain()

    async def recv_json():
        line = await proc.stdout.readline()
        if not line: return None
        return json.loads(line.decode())

    # Initialize
    await send_json({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "val-client", "version": "1.0.0"}}})
    await recv_json()
    await send_json({"jsonrpc": "2.0", "method": "notifications/initialized"})

    # Provision invalid board
    print("Provisioning invalid board (should fail)...")
    await send_json({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "provision_board",
            "arguments": {"node_id": "bad_node", "board_config": "this is not yaml: {{"}
        }
    })
    res = await recv_json()
    content = res["result"]["content"][0]["text"]
    assert "error" in content or "Error" in content
    print(f"Received expected error: {content}")

    proc.terminate()
    await proc.wait()
    print("Validation test passed!")

if __name__ == "__main__":
    asyncio.run(main())

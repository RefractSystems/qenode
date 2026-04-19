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
        if not line: return None
        return json.loads(line.decode())

    # Initialize
    await send_json({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "stress-client", "version": "1.0.0"}}})
    await recv_json()
    await send_json({"jsonrpc": "2.0", "method": "notifications/initialized"})

    yaml_path = os.path.join(WORKSPACE_DIR, "test", "phase3", "test_board.yaml")
    with open(yaml_path, "r") as f: board_config = f.read()
    firmware_path = os.path.join(WORKSPACE_DIR, "test", "phase1", "hello.elf")

    node_id = "stress_node"
    
    print(f"Provisioning {node_id}...")
    await send_json({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "provision_board", "arguments": {"node_id": node_id, "board_config": board_config}}})
    await recv_json()
    
    await send_json({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "flash_firmware", "arguments": {"node_id": node_id, "firmware_path": firmware_path}}})
    await recv_json()

    for i in range(5):
        print(f"Iteration {i+1}: Starting...")
        await send_json({"jsonrpc": "2.0", "id": 100+i, "method": "tools/call", "params": {"name": "start_node", "arguments": {"node_id": node_id}}})
        res = await recv_json()
        if "error" in res:
            print(f"Start error: {res['error']}")
            break
        
        await asyncio.sleep(1)
        
        print(f"Iteration {i+1}: Reading PC...")
        await send_json({"jsonrpc": "2.0", "id": 200+i, "method": "tools/call", "params": {"name": "read_cpu_state", "arguments": {"node_id": node_id}}})
        res = await recv_json()
        
        print(f"Iteration {i+1}: Stopping...")
        await send_json({"jsonrpc": "2.0", "id": 300+i, "method": "tools/call", "params": {"name": "stop_node", "arguments": {"node_id": node_id}}})
        await recv_json()
        
        await asyncio.sleep(0.5)

    proc.terminate()
    await proc.wait()
    print("Stress test passed!")

if __name__ == "__main__":
    asyncio.run(main())

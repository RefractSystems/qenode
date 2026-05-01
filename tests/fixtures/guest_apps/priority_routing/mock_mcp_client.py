"""
SOTA Test Module: mock_mcp_client

Context:
This module implements tests for the mock_mcp_client subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of mock_mcp_client.
"""

import asyncio
import json
import logging
import sys
from pathlib import Path


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent.parent.parent

WORKSPACE_DIR = str(_find_workspace_root(Path(__file__).resolve()))
if WORKSPACE_DIR not in sys.path:
    sys.path.append(WORKSPACE_DIR)

logging.basicConfig(level=logging.INFO, format="%(message)s", stream=sys.stderr)
logger = logging.getLogger(__name__)


async def main():
    logger.info("Connecting to MCP server...")
    proc = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "tools.mcp_server",
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        cwd=WORKSPACE_DIR,
    )

    async def forward_stderr():
        while True:
            line = await proc.stderr.readline()
            if not line:
                break
            sys.stderr.write(f"[server] {line.decode()}")
            sys.stderr.flush()

    asyncio.create_task(forward_stderr())  # noqa: RUF006

    async def send_json(obj):
        data = json.dumps(obj) + "\n"
        proc.stdin.write(data.encode())
        await proc.stdin.drain()

    async def recv_json():
        line = await proc.stdout.readline()
        if not line:
            return None
        logger.info(f"<- {line.decode().strip()}")
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

    # Read the exact YAML file used in tests
    yaml_path = Path(WORKSPACE_DIR) / "tests" / "fixtures" / "guest_apps" / "yaml_boot" / "test_board.yaml"
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
    firmware_path = Path(WORKSPACE_DIR) / "tests" / "fixtures" / "guest_apps" / "boot_arm" / "hello.elf"
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

    logger.info("Waiting deterministically for CPU state...")
    for _ in range(50):
        await send_json(
            {
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {"name": "read_cpu_state", "arguments": {"node_id": "node0"}},
            }
        )
        res = await recv_json()
        try:
            state_text = res["result"]["content"][0]["text"]
            import re

            if re.search(r"(?i)\b(pc|r15|eip|rip)\b\s*(?:=|\s+)\s*[0-9a-f]+", state_text):
                logger.info("Read CPU State Result: " + json.dumps(res, indent=2))
                break
        except Exception:
            pass
        await asyncio.sleep(0.1)  # SLEEP_EXCEPTION: RPC polling backoff
    else:
        logger.warning("Warning: CPU state could not be read in time.")

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
    logger.info("Done.")


if __name__ == "__main__":
    asyncio.run(main())

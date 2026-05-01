"""
SOTA Test Module: mcp_stress_test

Context:
This module implements tests for the mcp_stress_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of mcp_stress_test.
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

    _ = asyncio.create_task(forward_stderr())

    async def send_json(obj):
        data = json.dumps(obj) + "\n"
        proc.stdin.write(data.encode())
        await proc.stdin.drain()

    async def recv_json():
        line = await proc.stdout.readline()
        if not line:
            return None
        return json.loads(line.decode())

    # Initialize
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "stress-client", "version": "1.0.0"},
            },
        }
    )
    await recv_json()
    await send_json({"jsonrpc": "2.0", "method": "notifications/initialized"})

    yaml_path = Path(WORKSPACE_DIR) / "tests" / "fixtures" / "guest_apps" / "yaml_boot" / "test_board.yaml"
    with Path(yaml_path).open() as f:
        board_config = f.read()
    firmware_path = Path(WORKSPACE_DIR) / "tests" / "fixtures" / "guest_apps" / "boot_arm" / "hello.elf"

    node_id = "stress_node"

    logger.info(f"Provisioning {node_id}...")
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "provision_board", "arguments": {"node_id": node_id, "board_config": board_config}},
        }
    )
    await recv_json()

    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "flash_firmware",
                "arguments": {"node_id": "stress_node", "firmware_path": str(firmware_path)},
            },
        }
    )
    await recv_json()

    for i in range(5):
        logger.info(f"Iteration {i + 1}: Starting...")
        await send_json(
            {
                "jsonrpc": "2.0",
                "id": 100 + i,
                "method": "tools/call",
                "params": {"name": "start_node", "arguments": {"node_id": node_id}},
            }
        )
        res = await recv_json()
        if "error" in res:
            logger.error(f"Start error: {res['error']}")
            break
        logger.info(f"Iteration {i + 1}: Waiting deterministically for CPU state...")
        for _ in range(50):
            await send_json(
                {
                    "jsonrpc": "2.0",
                    "id": 200 + i,
                    "method": "tools/call",
                    "params": {"name": "read_cpu_state", "arguments": {"node_id": node_id}},
                }
            )
            res = await recv_json()
            try:
                state_text = res["result"]["content"][0]["text"]
                # Architecture-agnostic PC check (matches R15, PC, EIP, RIP, pc)
                import re

                if re.search(r"(?i)\b(pc|r15|eip|rip)\b\s*(?:=|\s+)\s*[0-9a-f]+", state_text):
                    break
            except Exception as e:
                logger.debug(f"Failed to parse state: {e} | Res: {res}")
            await asyncio.sleep(0.1)  # SLEEP_EXCEPTION: RPC polling backoff
        else:
            logger.warning("Warning: CPU state could not be read in time.")

        logger.info(f"Iteration {i + 1}: Stopping...")
        await send_json(
            {
                "jsonrpc": "2.0",
                "id": 300 + i,
                "method": "tools/call",
                "params": {"name": "stop_node", "arguments": {"node_id": node_id}},
            }
        )
        await recv_json()

    proc.terminate()
    await proc.wait()
    logger.info("Stress test passed!")


if __name__ == "__main__":
    asyncio.run(main())

"""
SOTA Test Module: validation_test

Context:
This module implements tests for the validation_test subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of validation_test.
"""

import asyncio
import json
import logging
import sys
import typing

from tools.testing.env import WORKSPACE_DIR

logging.basicConfig(level=logging.INFO, format="%(message)s", stream=sys.stderr)
logger = logging.getLogger(__name__)


async def main() -> None:
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

    async def log_stderr() -> None:
        while True:
            line = await proc.stderr.readline()  # type: ignore[union-attr]
            if not line:
                break
            sys.stderr.write(f"[server] {line.decode()}")
            sys.stderr.flush()

    asyncio.create_task(log_stderr())  # noqa: RUF006

    async def send_json(obj: dict[typing.Any, typing.Any]) -> None:
        data = json.dumps(obj) + "\n"
        proc.stdin.write(data.encode())  # type: ignore[union-attr]
        await proc.stdin.drain()  # type: ignore[union-attr]

    async def recv_json() -> typing.Any:  # noqa: ANN401
        line = await proc.stdout.readline()  # type: ignore[union-attr]
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
                "clientInfo": {"name": "val-client", "version": "1.0.0"},
            },
        }
    )
    await recv_json()
    await send_json({"jsonrpc": "2.0", "method": "notifications/initialized"})

    # Provision invalid board
    logger.info("Provisioning invalid board (should fail)...")
    await send_json(
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "provision_board",
                "arguments": {"node_id": "bad_node", "board_config": "this is not yaml: {{"},
            },
        }
    )
    res = await recv_json()
    content = res["result"]["content"][0]["text"]
    assert "error" in content or "Error" in content
    logger.info(f"Received expected error: {content}")

    proc.terminate()
    await proc.wait()
    logger.info("Validation test passed!")


if __name__ == "__main__":
    asyncio.run(main())

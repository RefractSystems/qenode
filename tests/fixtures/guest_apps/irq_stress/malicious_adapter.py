"""
SOTA Test Module: malicious_adapter

Context:
This module implements tests for the malicious_adapter subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of malicious_adapter.
"""

import logging
import socket
import sys
from pathlib import Path


def _find_workspace_root(start_path: Path) -> Path:
    for p in [start_path, *list(start_path.parents)]:
        if (p / "VERSION").exists() or (p / ".git").exists():
            return p
    return start_path.parent.parent.parent  # Fallback

SCRIPT_DIR = Path(__file__).resolve().parent
WORKSPACE_DIR = _find_workspace_root(Path(__file__).resolve())
TOOLS_DIR = WORKSPACE_DIR / "tools"

if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import vproto  # noqa: E402

logger = logging.getLogger(__name__)

# From virtmcu_proto.h
VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1


def main():
    if len(sys.argv) < 3:
        logger.info("Usage: malicious_adapter.py <socket_path> <mode>")
        logger.info("Modes: hang, crash")
        sys.exit(1)

    sock_path = sys.argv[1]
    mode = sys.argv[2]

    if Path(sock_path).exists():
        Path(sock_path).unlink()

    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(sock_path)
    server.listen(1)

    logger.info(f"Malicious adapter ({mode}) listening on {sock_path}")
    conn, _ = server.accept()
    logger.info("Connection accepted")

    # 1. Handshake
    data = conn.recv(8)
    if not data:
        logger.info("No data received for handshake")
        return
    hs = vproto.VirtmcuHandshake.unpack(data)
    logger.info(f"Received handshake: magic=0x{hs.magic:X}, version={hs.version}")

    hs_out = vproto.VirtmcuHandshake(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION).pack()
    conn.sendall(hs_out)
    logger.info("Sent handshake")

    # 2. Wait for first MMIO request
    # req_data = conn.recv(32) # mmio_req is 32 bytes
    req_data = b""
    while len(req_data) < 32:
        chunk = conn.recv(32 - len(req_data))
        if not chunk:
            break
        req_data += chunk

    if len(req_data) == 32:
        req_type = req_data[0]
        logger.info(f"Received MMIO request: type={req_type}")

        if mode == "hang":
            logger.info("Ignoring request to trigger timeout in QEMU...")
            # Keep the connection open but do nothing
            while True:
                try:
                    if not conn.recv(1024):
                        break
                except Exception:
                    break
        elif mode == "crash":
            logger.info("Closing connection immediately to simulate crash...")
            conn.close()
        else:
            logger.info(f"Unknown mode: {mode}")

    conn.close()
    server.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()

import logging
import socket
import sys
from pathlib import Path

# Add tools directory to sys.path to allow importing vproto
SCRIPT_DIR = Path(__file__).resolve().parent
TOOLS_DIR = SCRIPT_DIR.parent.parent / "tools"
if str(TOOLS_DIR) not in sys.path:
    sys.path.append(str(TOOLS_DIR))

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

    logger.info(f"Malicious adapter ({mode}) listening on {sock_path}", flush=True)
    conn, _ = server.accept()
    logger.info("Connection accepted", flush=True)

    # 1. Handshake
    data = conn.recv(8)
    if not data:
        logger.info("No data received for handshake", flush=True)
        return
    hs = vproto.VirtmcuHandshake.unpack(data)
    logger.info(f"Received handshake: magic=0x{hs.magic:X}, version={hs.version}", flush=True)

    hs_out = vproto.VirtmcuHandshake(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION).pack()
    conn.sendall(hs_out)
    logger.info("Sent handshake", flush=True)

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
        logger.info(f"Received MMIO request: type={req_type}", flush=True)

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

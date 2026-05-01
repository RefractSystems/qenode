"""
SOTA Test Module: zenoh_router

Context:
This module implements tests for the zenoh_router subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of zenoh_router.
"""

import logging
import sys
import time

import zenoh

logger = logging.getLogger(__name__)


def main():
    if len(sys.argv) > 1:
        endpoint = sys.argv[1]
    else:
        # Resolve dynamic IP for default
        try:
            import subprocess
            from pathlib import Path

            def _find_workspace_root(start_path: Path) -> Path:
                for p in [start_path, *list(start_path.parents)]:
                    if (p / "VERSION").exists() or (p / ".git").exists():
                        return p
                return start_path.parent.parent.parent.parent

            workspace_dir = _find_workspace_root(Path(__file__).resolve())
            get_ip_script = workspace_dir / "scripts" / "get-free-port.py"
            host_ip = subprocess.check_output([sys.executable, str(get_ip_script), "--ip"]).decode().strip()
            endpoint = f"tcp/{host_ip}:7448"
        except Exception:
            endpoint = "tcp/localhost:7448"

    config = zenoh.Config()
    config.insert_json5("listen/endpoints", f'["{endpoint}"]')
    config.insert_json5("scouting/multicast/enabled", "false")

    logger.info(f"Starting Zenoh router on {endpoint}...")
    session = zenoh.open(config)

    logger.info("Router running. Press Ctrl+C to exit.")
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    session.close()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()

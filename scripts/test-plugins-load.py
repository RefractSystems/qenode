#!/usr/bin/env python3
"""
scripts/test-plugins-load.py

Smoke test for QEMU plugins:
1. Parse hw/meson.build to find all expected VirtMCU objects.
2. Boot a minimal QEMU machine with QMP enabled.
3. Query QOM types via QMP.
4. Assert all expected objects are registered as QOM types.
"""

import json
import logging
import os
import re
import socket
import subprocess
import sys
import time

from tools.testing.env import WORKSPACE_DIR

logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")
logger = logging.getLogger(__name__)


def get_free_port() -> int:
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def main() -> int:
    qemu_bin = WORKSPACE_DIR / "third_party/qemu/build-virtmcu/install/bin/qemu-system-arm"
    if not qemu_bin.exists():
        logger.error(f"QEMU binary not found at {qemu_bin}. Run 'make build' first.")
        return 1

    meson_build = WORKSPACE_DIR / "hw/meson.build"
    if not meson_build.exists():
        logger.error("hw/meson.build not found.")
        return 1

    with meson_build.open() as f:
        content = f.read()

    expected_objs = set()
    for match in re.finditer(r"'obj'\s*:\s*'([^']+)'", content):
        obj_name = match.group(1)
        # Skip objects that are known not to be direct QOM type names or are internal
        if obj_name in ["dummy", "remote-port"]:
            continue
        expected_objs.add(obj_name)

    logger.info(f"Expected VirtMCU objects from meson.build: {sorted(expected_objs)}")

    port = get_free_port()
    # Use any existing valid DTB for a minimal boot
    dtb_path = WORKSPACE_DIR / "tests/fixtures/guest_apps/boot_arm/minimal.dtb"
    if not dtb_path.exists():
        # Fallback to flexray one if boot_arm isn't built
        dtb_path = WORKSPACE_DIR / "tests/fixtures/guest_apps/flexray_bridge/platform.dtb"

    if not dtb_path.exists():
        logger.error("No valid DTB found for smoke test. Run 'make build-test-artifacts' first.")
        return 1

    cmd = [
        str(qemu_bin),
        "-machine",
        f"arm-generic-fdt,hw-dtb={dtb_path}",
        "-nographic",
        "-monitor",
        "none",
        "-display",
        "none",
        "-S",  # Don't actually boot
        "-qmp",
        f"tcp:127.0.0.1:{port},server,nowait",
    ]

    # Explicitly point to the local build's module directory
    module_dir = WORKSPACE_DIR / "third_party/qemu/build-virtmcu/install/lib/aarch64-linux-gnu/qemu"
    if not module_dir.exists():
        # Handle different architectures/paths if needed
        module_dir = WORKSPACE_DIR / "third_party/qemu/build-virtmcu/install/lib/x86_64-linux-gnu/qemu"

    env = os.environ.copy()
    env["QEMU_MODULE_DIR"] = str(module_dir.absolute())

    logger.info(f"Starting QEMU with QMP on port {port}...")
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    try:
        # Polling for QEMU to start and open the port
        connected = False
        for _ in range(10):
            time.sleep(0.5)
            if proc.poll() is not None:
                _, stderr = proc.communicate()
                logger.error(f"QEMU exited unexpectedly with code {proc.returncode}")
                logger.error(f"Stderr: {stderr.decode()}")
                return 1
            try:
                s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                s.settimeout(1.0)
                s.connect(("127.0.0.1", port))
                connected = True
                break
            except OSError:
                continue

        if not connected:
            logger.error("Timed out waiting for QEMU QMP connection.")
            return 1

        f = s.makefile("rw", encoding="utf-8")
        # Read greeting
        json.loads(f.readline())

        # Send qmp_capabilities
        f.write('{"execute": "qmp_capabilities"}\n')
        f.flush()
        json.loads(f.readline())

        # Send qom-list-types
        f.write('{"execute": "qom-list-types"}\n')
        f.flush()
        resp = json.loads(f.readline())

        types = [t["name"] for t in resp.get("return", [])]
        logger.info(f"Found {len(types)} QOM types.")

        missing = []
        for obj in expected_objs:
            # Type names might have 'virtmcu,' prefix or other variations?
            # Actually meson 'obj' usually matches the QOM type name exactly in our project
            if obj not in types and f"virtmcu,{obj}" not in types and f"zenoh-{obj}" not in types:
                missing.append(obj)

        if missing:
            logger.error(f"FAILED: The following VirtMCU objects failed to register as QOM types: {missing}")
            return 1

        logger.info("✅ All expected VirtMCU plugins loaded and registered successfully.")
        return 0

    except Exception as e:
        logger.error(f"Unexpected error during smoke test: {e}")
        raise
    finally:
        import contextlib

        with contextlib.suppress(Exception):
            f.write('{"execute": "quit"}\n')
            f.flush()
        proc.wait(timeout=5)
        if proc.poll() is None:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())

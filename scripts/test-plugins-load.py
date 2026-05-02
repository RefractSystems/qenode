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
from pathlib import Path

from tools.testing.env import WORKSPACE_DIR

logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")
logger = logging.getLogger(__name__)


def get_free_port() -> int:
    script = WORKSPACE_DIR / "scripts/get-free-port.py"
    return int(subprocess.check_output([sys.executable, str(script)]).decode().strip())


def main() -> int:
    qemu_bin = WORKSPACE_DIR / "third_party/qemu/build-virtmcu/install/bin/qemu-system-arm"
    module_dir = WORKSPACE_DIR / "third_party/qemu/build-virtmcu/install/lib/aarch64-linux-gnu/qemu"

    if not qemu_bin.exists():
        logger.info("Local QEMU build not found, trying /opt/virtmcu...")
        qemu_bin = Path("/opt/virtmcu/bin/qemu-system-arm")
        module_dir = Path("/opt/virtmcu/lib/aarch64-linux-gnu/qemu")
        if not module_dir.exists():
            module_dir = Path("/opt/virtmcu/lib/x86_64-linux-gnu/qemu")

    if not qemu_bin.exists():
        logger.error("QEMU binary not found. Run 'make build' first.")
        return 1

    meson_build = WORKSPACE_DIR / "hw/meson.build"
    if not meson_build.exists():
        logger.error("hw/meson.build not found.")
        return 1

    with meson_build.open() as f:
        content = f.read()

    expected_objs = set()
    # Match single 'obj': 'name'
    for match in re.finditer(r"'obj'\s*:\s*'([^']+)'", content):
        obj_name = match.group(1)
        if obj_name:
            expected_objs.add(obj_name)

    # Match 'objs': ['name1', 'name2']
    for match in re.finditer(r"'objs'\s*:\s*\[([^\]]+)\]", content):
        objs_str = match.group(1)
        for obj_name in re.findall(r"'([^']+)'", objs_str):
            if obj_name:
                expected_objs.add(obj_name)

    # Also look for manual additions like educational-dummy -> dummy-device
    if "educational-dummy" in content:
        expected_objs.add("dummy-device")

    # Filter out known non-QOM names or internal ones
    expected_objs = {obj for obj in expected_objs if obj not in ["dummy", "remote-port"]}

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

    env = os.environ.copy()
    env["QEMU_MODULE_DIR"] = str(module_dir.absolute())

    logger.info(f"Starting QEMU with QMP on port {port}...")
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    qmp_file = None
    try:
        # Polling for QEMU to start and open the port
        connected = False
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        for _ in range(20):  # Increased polling
            if proc.poll() is not None:
                stdout, stderr = proc.communicate()
                logger.error(f"QEMU exited unexpectedly with code {proc.returncode}")
                logger.error(f"Stdout: {stdout.decode()}")
                logger.error(f"Stderr: {stderr.decode()}")
                return 1
            try:
                s.settimeout(1.0)
                s.connect(("127.0.0.1", port))
                connected = True
                break
            except OSError:
                continue

        if not connected:
            logger.error("Timed out waiting for QEMU QMP connection.")
            return 1

        s.settimeout(10.0)  # 10s timeout for commands
        qmp_file = s.makefile("rw", encoding="utf-8")
        # Read greeting
        json.loads(qmp_file.readline())

        # Send qmp_capabilities
        qmp_file.write('{"execute": "qmp_capabilities"}\n')
        qmp_file.flush()
        json.loads(qmp_file.readline())

        # Send qom-list-types
        qmp_file.write('{"execute": "qom-list-types"}\n')
        qmp_file.flush()
        resp = json.loads(qmp_file.readline())

        types = [t["name"] for t in resp.get("return", [])]
        logger.info(f"Found {len(types)} QOM types.")

        sim_types = [t for t in types if "virtmcu" in t.lower() or "zenoh" in t.lower()]
        logger.debug(f"Sim-related types found: {sorted(sim_types)}")

        missing = []
        for obj in expected_objs:
            # Check for various name variations due to transitions (zenoh- vs virtmcu- prefix)
            found = False
            variations = {
                obj,
                f"virtmcu,{obj}",
                f"zenoh-{obj}",
                f"virtmcu-{obj}",
                obj.replace("virtmcu-", "zenoh-"),
                obj.replace("zenoh-", "virtmcu-"),
                obj.replace("-virtmcu", "-zenoh"),
                obj.replace("-zenoh", "-virtmcu"),
                # Handle ieee802154 -> 802154 mapping seen in some versions
                obj.replace("ieee", ""),
                f"zenoh-{obj.replace('ieee', '')}",
                f"virtmcu-{obj.replace('ieee', '')}",
            }
            for var in variations:
                if var in types:
                    found = True
                    break

            if not found:
                missing.append(obj)

        if missing:
            logger.error(f"FAILED: The following VirtMCU objects failed to register as QOM types: {missing}")
            logger.info(f"Registered sim types were: {sorted(sim_types)}")
            return 1

        logger.info("✅ All expected VirtMCU plugins loaded and registered successfully.")
        return 0

    except Exception as e:  # noqa: BLE001
        logger.error(f"Unexpected error during smoke test: {e}")
        return 1
    finally:
        if qmp_file:
            try:
                qmp_file.write('{"execute": "quit"}\n')
                qmp_file.flush()
            except Exception:  # noqa: BLE001, S110
                pass

        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


if __name__ == "__main__":
    sys.exit(main())

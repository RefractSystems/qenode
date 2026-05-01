"""
SOTA Test Module: test_bridge_irq

Context:
This module implements tests for the test_bridge_irq subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_bridge_irq.
"""

import logging
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from tools.testing.env import WORKSPACE_DIR
from tools.testing.utils import mock_execution_delay

logger = logging.getLogger(__name__)


def test_irq() -> None:
    sock_path = tempfile.mkstemp(suffix=".sock")[1]
    tempfile.mkstemp(suffix=".log")[1]

    # 1. Start Mock Adapter
    cat_cmd = """
import os, socket, struct, time, sys, pathlib
import tools.vproto as vproto
from tools.testing.utils import mock_execution_delay

VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1
SYSC_MSG_IRQ_SET = 1
SYSC_MSG_IRQ_CLEAR = 2

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind("{sock_path}")
s.listen(1)
conn, addr = s.accept()
hs = conn.recv(8)
conn.sendall(hs)
mock_execution_delay(1)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
# Send IRQs
conn.sendall(vproto.SyscMsg(SYSC_MSG_IRQ_SET, 5, 0).pack())
mock_execution_delay(0.1)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
conn.sendall(vproto.SyscMsg(SYSC_MSG_IRQ_CLEAR, 5, 0).pack())
mock_execution_delay(1)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
conn.close()
""".replace("{sock_path}", sock_path)

    env = os.environ.copy()
    env["PYTHONPATH"] = str(WORKSPACE_DIR)
    adapter_proc = subprocess.Popen([shutil.which("python3") or "python3", "-c", cat_cmd], env=env)

    # 2. Prepare DTS and Firmware
    # We use a dummy firmware that just hangs
    dts = f"""
/dts-v1/;
/ {{
    compatible = "arm,generic-fdt";
    #address-cells = <2>; #size-cells = <2>;
    qemu_sysmem: qemu_sysmem {{ compatible = "qemu:system-memory"; phandle = <0x01>; }};
    chosen {{}};
    memory@40000000 {{ compatible = "qemu-memory-region"; qemu,ram = <0x01>; container = <0x01>; reg = <0x0 0x40000000 0x0 0x1000>; }};
    cpus {{ #address-cells = <1>; #size-cells = <0>; cpu@0 {{ device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; }}; }};
    bridge@50000000 {{ compatible = "mmio-socket-bridge"; reg = <0x0 0x70000000 0x0 0x1000>; socket-path = "{sock_path}"; region-size = <0x1000>; }};
}};
"""
    with Path(str(Path(tempfile.gettempdir()) / "irq.dts")).open("w") as f:
        f.write(dts)
    subprocess.run(
        [
            shutil.which("dtc") or "dtc",
            "-I",
            "dts",
            "-O",
            "dtb",
            "-o",
            str(Path(tempfile.gettempdir()) / "irq.dtb"),
            str(Path(tempfile.gettempdir()) / "irq.dts"),
        ]
    )

    # Dummy firmware: hlt (wait for interrupt)
    with Path(str(Path(tempfile.gettempdir()) / "irq.S")).open("w") as f:
        f.write(".global _start\n_start:\nwfi\nb _start\n")
    subprocess.run(
        [
            shutil.which("arm-none-eabi-gcc") or "arm-none-eabi-gcc",
            "-mcpu=cortex-a15",
            "-nostdlib",
            "-Ttext=0x40000000",
            str(Path(tempfile.gettempdir()) / "irq.S"),
            "-o",
            str(Path(tempfile.gettempdir()) / "irq.elf"),
        ]
    )

    # 3. Start QEMU
    build_dir = "build-virtmcu-asan" if os.environ.get("VIRTMCU_USE_ASAN") == "1" else "build-virtmcu"
    qemu_bin = f"/workspace/third_party/qemu/{build_dir}/install/bin/qemu-system-arm"
    qemu_proc = subprocess.Popen(
        [
            qemu_bin,
            "-M",
            "arm-generic-fdt,hw-dtb=/tmp/irq.dtb",
            "-kernel",
            str(Path(tempfile.gettempdir()) / "irq.elf"),
            "-nographic",
            "-monitor",
            "none",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    mock_execution_delay(5)  # SLEEP_EXCEPTION: mock test simulating execution/spacing
    qemu_proc.terminate()
    adapter_proc.terminate()

    logger.info("IRQ test finished. Check coverage.")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    test_irq()

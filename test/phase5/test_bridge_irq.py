import os
import subprocess
import tempfile
import time
from pathlib import Path


def test_irq():
    sock_path = tempfile.mktemp(suffix=".sock")
    tempfile.mktemp(suffix=".log")

    # 1. Start Mock Adapter
    cat_cmd = f"""
import os, socket, struct, time
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
time.sleep(1)
# Send IRQs
conn.sendall(struct.pack("<IIQ", SYSC_MSG_IRQ_SET, 5, 0))
time.sleep(0.1)
conn.sendall(struct.pack("<IIQ", SYSC_MSG_IRQ_CLEAR, 5, 0))
time.sleep(1)
conn.close()
"""
    adapter_proc = subprocess.Popen(["python3", "-c", cat_cmd])

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
    with Path("/tmp/irq.dts").open("w") as f:
        f.write(dts)
    subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", "/tmp/irq.dtb", "/tmp/irq.dts"])

    # Dummy firmware: hlt (wait for interrupt)
    with Path("/tmp/irq.S").open("w") as f:
        f.write(".global _start\n_start:\nwfi\nb _start\n")
    subprocess.run(
        ["arm-none-eabi-gcc", "-mcpu=cortex-a15", "-nostdlib", "-Ttext=0x40000000", "/tmp/irq.S", "-o", "/tmp/irq.elf"]
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
            "/tmp/irq.elf",
            "-nographic",
            "-monitor",
            "none",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    time.sleep(5)
    qemu_proc.terminate()
    adapter_proc.terminate()

    print("IRQ test finished. Check coverage.")


if __name__ == "__main__":
    test_irq()

import subprocess
import tempfile
import time


def test_handshake_fail():
    sock_path = tempfile.mktemp(suffix=".sock")

    # Adapter sends wrong magic
    cat_cmd = f"""
import os, socket, struct
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind("{sock_path}")
s.listen(1)
conn, addr = s.accept()
hs = conn.recv(8)
# Send wrong magic
conn.sendall(struct.pack("<II", 0xDEADBEEF, 1))
conn.close()
"""
    adapter_proc = subprocess.Popen(["python3", "-c", cat_cmd])

    dts = f"""
/dts-v1/;
/ {{
    compatible = "arm,generic-fdt";
    #address-cells = <2>; #size-cells = <2>;
    qemu_sysmem: qemu_sysmem {{ compatible = "qemu:system-memory"; phandle = <0x01>; }};
    chosen {{}};
    memory@40000000 {{ compatible = "qemu-memory-region"; qemu,ram = <0x01>; container = <0x01>; reg = <0x0 0x40000000 0x0 0x1000>; }};
    cpus {{ #address-cells = <1>; #size-cells = <0>; cpu@0 {{ device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; }}; }};
    bridge@50000000 {{ compatible = "mmio-socket-bridge"; reg = <0x0 0x50000000 0x0 0x1000>; socket-path = "{sock_path}"; region-size = <0x1000>; reconnect-ms = <1000>; }};
}};
"""
    with open("/tmp/handshake.dts", "w") as f: f.write(dts)
    subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", "/tmp/handshake.dtb", "/tmp/handshake.dts"])

    with open("/tmp/dummy.S", "w") as f: f.write(".global _start\n_start: b _start\n")
    subprocess.run(["arm-none-eabi-gcc", "-mcpu=cortex-a15", "-nostdlib", "-Ttext=0x40000000", "/tmp/dummy.S", "-o", "/tmp/dummy.elf"])

    qemu_proc = subprocess.Popen([
        "/workspace/third_party/qemu/build-virtmcu/install/bin/qemu-system-arm",
        "-M", "arm-generic-fdt,hw-dtb=/tmp/handshake.dtb",
        "-kernel", "/tmp/dummy.elf",
        "-nographic", "-monitor", "none"
    ])

    time.sleep(3)
    qemu_proc.terminate()
    adapter_proc.terminate()
    print("Handshake test finished.")

if __name__ == "__main__":
    test_handshake_fail()

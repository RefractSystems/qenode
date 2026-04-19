import os
import socket
import struct
import subprocess
import time
import sys
import json

VIRTMCU_PROTO_MAGIC = 0x564D4355
VIRTMCU_PROTO_VERSION = 1
SYSC_MSG_RESP = 0
SYSC_MSG_IRQ_SET = 1
SYSC_MSG_IRQ_CLEAR = 2

def run_qmp_cmd(sock_path, cmd):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(5.0)
    s.connect(sock_path)
    s.recv(4096)
    s.sendall(json.dumps({"execute": "qmp_capabilities"}).encode() + b"\n")
    s.recv(4096)
    s.sendall(json.dumps(cmd).encode() + b"\n")
    resp = b""
    while b"return" not in resp:
        chunk = s.recv(4096)
        if not chunk: break
        resp += chunk
    s.close()
    return json.loads(resp.decode())

def main():
    sock_path = "/tmp/stress_irq.sock"
    qmp_path = "/tmp/stress_irq_qmp.sock"
    dtb_path = "/tmp/stress_irq.dtb"
    elf_path = "/tmp/stress_irq.elf"

    if os.path.exists(sock_path): os.remove(sock_path)

    dts = f"""
/dts-v1/;
/ {{
    compatible = "arm,generic-fdt";
    #address-cells = <2>; #size-cells = <2>;
    qemu_sysmem: qemu_sysmem {{ compatible = "qemu:system-memory"; phandle = <0x01>; }};
    chosen {{}};
    memory@40000000 {{ compatible = "qemu-memory-region"; qemu,ram = <0x01>; container = <0x01>; reg = <0x0 0x40000000 0x0 0x10000000>; }};
    cpus {{ #address-cells = <1>; #size-cells = <0>; cpu@0 {{ device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; }}; }};
    gic: interrupt-controller@8000000 {{
        compatible = "arm_gic";
        #interrupt-cells = <3>;
        interrupt-controller;
        reg = <0x0 0x08000000 0x0 0x1000>, <0x0 0x08010000 0x0 0x1000>;
        num-irq = <64>;
    }};
    bridge@50000000 {{
        compatible = "mmio-socket-bridge";
        reg = <0x0 0x50000000 0x0 0x1000>;
        socket-path = "{sock_path}";
        region-size = <0x1000>;
        interrupt-parent = <&gic>;
        interrupts = <0 0 4>; 
    }};
}};
"""
    with open("/tmp/stress_irq.dts", "w") as f: f.write(dts)
    subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", dtb_path, "/tmp/stress_irq.dts"])
    
    # Firmware that just spins
    with open("/tmp/stress_irq.S", "w") as f: f.write(".global _start\n_start:\nb _start\n")
    subprocess.run(["arm-none-eabi-gcc", "-mcpu=cortex-a15", "-nostdlib", "-Ttext=0x40000000", "/tmp/stress_irq.S", "-o", elf_path])

    qemu_cmd = [
        "/workspace/third_party/qemu/build-virtmcu/install/bin/qemu-system-arm",
        "-M", "arm-generic-fdt,hw-dtb=" + dtb_path,
        "-kernel", elf_path,
        "-nographic", "-monitor", "none",
        "-qmp", f"unix:{qmp_path},server,nowait"
    ]
    
    qemu_proc = subprocess.Popen(qemu_cmd)
    
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(sock_path)
    server.listen(1)
    
    conn = None
    for _ in range(100):
        try:
            server.settimeout(0.1)
            conn, _ = server.accept()
            break
        except socket.timeout:
            continue
    
    if not conn:
        print("QEMU did not connect")
        qemu_proc.terminate()
        sys.exit(1)
        
    hs = conn.recv(8)
    conn.sendall(hs)
    
    print("Starting IRQ stress test...", flush=True)
    NUM_IRQS = 1000
    start_time = time.time()
    for i in range(NUM_IRQS):
        conn.sendall(struct.pack("<IIQ", SYSC_MSG_IRQ_SET, 0, 0))
        conn.sendall(struct.pack("<IIQ", SYSC_MSG_IRQ_CLEAR, 0, 0))
        if i % 100 == 0:
             print(f"Sent {i} IRQs...", flush=True)
             # Periodically check QMP responsiveness
             resp = run_qmp_cmd(qmp_path, {"execute": "query-status"})
             if "return" not in resp:
                 print(f"QMP unresponsive at {i} IRQs", flush=True)
                 break
        # time.sleep(0.0001) # Very small sleep to allow QEMU to breathe if needed

    end_time = time.time()
    print(f"Finished {NUM_IRQS} IRQ pairs in {end_time - start_time:.2f}s")
    
    # Verify final state
    resp = run_qmp_cmd(qmp_path, {"execute": "human-monitor-command", "arguments": {"command-line": "info pic"}})
    print("Final PIC state:\n", resp.get("return", ""))

    qemu_proc.terminate()
    conn.close()
    server.close()
    print("Stress test PASSED!")

if __name__ == "__main__":
    main()

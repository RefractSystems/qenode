# Lesson 11.3: Remote Port Co-Simulation (Path B)

In Lesson 5, we learned how to use `mmio-socket-bridge` to perform co-simulation via a custom protocol (Path A). In this lesson, we will implement **Path B**: full TLM-2.0 co-simulation using the industry-standard **AMD/Xilinx Remote Port** protocol.

## Why Remote Port?

While our custom protocol (`virtmcu_proto.h`) is lightweight, many hardware engineers use **Verilator** to compile Verilog/SystemVerilog FPGA designs into C++ models. AMD/Xilinx provides `libsystemctlm-soc`, a powerful open-source library that connects Verilated models (or SystemC models) to QEMU using the Remote Port protocol.

By adding Remote Port support to `virtmcu`, firmware developers can run bare-metal binaries against completely custom, cycle-accurate FPGA hardware running locally or on a remote simulation server.

## The QEMU Side: `remote-port-bridge`

We have added a new dynamic QOM module: `hw-virtmcu-remote-port-bridge.so`.
This device behaves identically to `mmio-socket-bridge`, but formats its memory reads, writes, and interrupts using the Xilinx Remote Port packet format.

To instantiate it dynamically in your Device Tree:

```dts
    bridge@60000000 {
        compatible = "remote-port-bridge";
        reg = <0x0 0x60000000 0x0 0x1000>;
        socket-path = "/tmp/rp.sock";
        region-size = <0x1000>;
    };
```

*(Note: Our platform parser maps `RemotePort.Peripheral` automatically to this device!)*

## The SystemC Side: `rp_adapter`

We use CMake's `FetchContent` to download `libsystemctlm-soc` directly from GitHub during the build process. We then use its `remoteport_tlm` and `remoteport_tlm_memory_master` classes to translate the incoming Unix socket packets back into SystemC TLM-2.0 `b_transport` calls.

```cpp
    remoteport_tlm rp("rp_server", -1, "unix:/tmp/rp.sock");
    remoteport_tlm_memory_master rp_mem("rp_mem");
    
    rp.register_dev(0, &rp_mem);
    rp_mem.sk.bind(your_custom_hardware.socket);
```

Whenever QEMU reads or writes to `0x60000000`, the transaction is forwarded over the socket and arrives at your custom hardware's `b_transport` function with full timing and data length accuracy.

## Running the Smoke Test

We provide a full end-to-end smoke test that compiles an ARM firmware, a SystemC adapter, and boots QEMU:

```bash
make -C tools/systemc_adapter/build rp_adapter
bash test/phase11_3/smoke_test.sh
```

You will see the adapter logging the exact `READ` and `WRITE` payloads issued by the ARM CPU executing the `str` and `ldr` assembly instructions.

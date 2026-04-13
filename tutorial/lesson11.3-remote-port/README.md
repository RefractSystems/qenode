# Lesson 11.3: Remote Port Co-Simulation (Path B)

## Background: Two Co-simulation Paths in virtmcu

In Lesson 5 we built **Path A** co-simulation: a custom binary protocol over a Unix socket,
implemented in `hw/misc/mmio-socket-bridge.c`.  Path A is simple and fast, but it speaks
only to adapters written specifically for virtmcu.

**Path B** uses the **AMD/Xilinx Remote Port** protocol — the same binary wire format used
by Xilinx's own QEMU fork, Renode's co-simulation bridge, and `libsystemctlm-soc`.  Any
tool that already speaks Remote Port (Verilator wrappers, SystemC TLM-2.0 models, PYNQ,
Vitis HLS test benches) can connect to a virtmcu simulation with zero protocol changes.

---

## The Wire Protocol

Remote Port is a simple request/response protocol over a reliable stream socket (TCP or
Unix domain).  Every transaction starts with a header:

```c
struct rp_pkt_hdr {
    uint32_t cmd;    /* RP_CMD_read, RP_CMD_write, RP_CMD_interrupt, … */
    uint32_t len;    /* payload bytes following the header */
    uint32_t id;     /* transaction ID — echoed in the response */
    uint32_t dev;    /* target device index on the remote side */
    uint32_t flags;  /* RP_PKT_FLAGS_response set in reply packets */
} __attribute__((packed));
```

A `HELLO` handshake is exchanged first to negotiate the protocol version (currently 4.3).
Reads and writes then follow a symmetric request/response pattern: QEMU sends a
`RP_CMD_write` packet with the data payload appended; the remote side echoes a
`RP_CMD_write` response (with `RP_PKT_FLAGS_response`) to acknowledge receipt.

---

## The QEMU Side: `hw/remote-port/remote-port-bridge.c`

This is a QOM `SysBusDevice` compiled as `hw-virtmcu-remote-port-bridge.so`.  It:

1. Connects to a Unix-domain socket at realize time and performs the HELLO handshake.
2. Registers a non-blocking `qemu_set_fd_handler` receive callback (`bridge_sock_handler`)
   that runs in QEMU's main event loop.
3. Exposes an MMIO region whose `read`/`write` ops call `send_req_and_wait`.

### BQL Discipline in `send_req_and_wait`

QEMU's Big QEMU Lock (BQL) protects all device state.  The vCPU thread holds it when
executing MMIO handlers.  If we block waiting for the remote socket response while holding
the BQL, the main event loop — which also needs the BQL to run our receive callback — will
deadlock.

The correct unlock sequence is:

```c
/* 1. Lock the socket mutex FIRST, before releasing BQL.
 *    This prevents the main loop from running bridge_sock_handler()
 *    and setting has_resp=true before we reset it below.       */
qemu_mutex_lock(&s->sock_mutex);
s->has_resp = false;

/* 2. Release BQL so the main event loop can call bridge_sock_handler. */
bql_unlock();

/* 3. Send the request, then sleep on the condition variable.
 *    qemu_cond_wait atomically releases sock_mutex and suspends.  */
writen(s->sock_fd, req, req_len);
while (!s->has_resp) {
    qemu_cond_wait(&s->resp_cond, &s->sock_mutex);
}

/* 4. Re-acquire BQL before releasing the socket mutex so no other
 *    thread can observe partially-restored device state.         */
bql_lock();
qemu_mutex_unlock(&s->sock_mutex);
```

If the order were `bql_unlock() → qemu_mutex_lock()`, the main loop could run
`bridge_sock_handler()` in the gap, see the _previous_ value of `has_resp`, and signal
before we reset the flag — causing the vCPU thread to wait forever.

### Device Tree instantiation

```dts
bridge@60000000 {
    compatible = "remote-port-bridge";
    reg = <0x0 0x60000000 0x0 0x1000>;
    socket-path = "/tmp/rp.sock";
    region-size = <0x1000>;
};
```

`socket-path` and `region-size` are required properties.  `base-addr` defaults to the
address in `reg` when arm-generic-fdt maps the node.

---

## The SystemC Side: `tools/systemc_adapter/remote_port_adapter.cpp`

`rp_adapter` uses `libsystemctlm-soc` (fetched via CMake `FetchContent`) to implement the
server side of the Remote Port protocol.  It creates three SystemC modules:

| Module | Role |
|--------|------|
| `remoteport_tlm rp` | Accepts the HELLO, listens on the socket, dispatches packets |
| `remoteport_tlm_memory_master rp_mem` | Converts RP read/write packets to TLM-2.0 `b_transport` calls |
| `RegisterFile regfile` | Your custom hardware — replace this with a Verilated model |

Wiring is three lines:

```cpp
rp.register_dev(0, &rp_mem);    // device index 0 ↔ QEMU bridge dev=0
rp_mem.sk.bind(regfile.socket); // TLM-2.0 socket → your hardware
```

To attach a **Verilator model** instead of `RegisterFile`, bind `rp_mem.sk` to the
Verilated module's TLM target socket — Verilator generates these when you annotate the
SystemVerilog with `// verilator public`.

---

## Building the Adapter

The adapter requires SystemC and `libsystemctlm-soc`.  Both are fetched automatically by
CMake:

```bash
# Configure once
cmake -S tools/systemc_adapter -B tools/systemc_adapter/build -DCMAKE_BUILD_TYPE=Release

# Build
make -C tools/systemc_adapter/build rp_adapter
```

The `build/` directory is gitignored; you must run `cmake` on every fresh checkout.

---

## Running the Smoke Test

The smoke test compiles a minimal ARM firmware, starts the SystemC adapter, boots QEMU,
and asserts that the adapter received the expected MMIO writes:

```bash
bash test/phase11_3/smoke_test.sh
```

Expected output (abbreviated):

```
[rp_adapter] WRITE to addr=0x0 val=0xefbeadde len=4   # 0xdeadbeef in little-endian
[rp_adapter] READ  from addr=0x0 len=4
[rp_adapter] WRITE to addr=0x4 val=0x44332211 len=4
✓ Phase 11.3 smoke test PASSED
```

The firmware executes these ARM instructions:

```asm
ldr r0, =0x60000000   @ bridge base
ldr r1, =0xdeadbeef
str r1, [r0]          @ write → RP_CMD_write → TLM_WRITE_COMMAND
ldr r2, [r0]          @ read  → RP_CMD_read  → TLM_READ_COMMAND
```

The data bytes appear in little-endian order in the adapter log because ARM stores
multi-byte values with the least-significant byte at the lowest address.

---

## Connecting a Real Verilator Model

Replace `RegisterFile` in `remote_port_adapter.cpp` with your Verilated module:

```cpp
#include "Vmydesign.h"   // generated by Verilator

class VerilatedHW : public sc_module {
public:
    tlm_utils::simple_target_socket<VerilatedHW> socket;
    Vmydesign *dut;

    void b_transport(tlm::tlm_generic_payload& trans, sc_time& delay) {
        // map TLM transaction → DUT port writes, tick the clock
    }
    // …
};
```

The Remote Port protocol carries full address, byte-enable, and burst-length information
in `rp_pkt_busaccess`, so you have everything needed for a cycle-accurate register model.

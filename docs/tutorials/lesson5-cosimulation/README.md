# Lesson 5: Hardware Co-Simulation — Connecting SystemC Models to QEMU

In this lesson you will learn how QEMU's MMIO subsystem can be extended to
communicate with an external hardware model — specifically a **SystemC TLM-2.0**
register file — over a Unix domain socket.  This is the *Path A* co-simulation
strategy described in `docs/architecture/01-system-overview.md §9`.

## Concepts

### Co-simulation: why it matters

A peripheral model baked into QEMU needs to be re-compiled every time it
changes.  If the model already exists as a SystemC (or C++) component — for
example, a Verilated RTL block from a hardware team — you do not want to port
it to QEMU's C API.  Instead, you run the model as a separate process and have
QEMU forward MMIO accesses to it over an IPC channel.

### Path A — Unix socket bridge

```
Firmware
  │  ld/str (ARM MMIO)
  ▼
QEMU MemoryRegion (mmio-socket-bridge)
  │  struct mmio_req  [32 bytes]
  │  ─────────────► Unix Domain Socket ─────────────►
  │                                                  SystemC QemuAdapter
  │                                                      │  b_transport()
  │                                                      ▼
  │                                                  RegisterFile
  │                                                      │
  │  struct sysc_msg [ 16 bytes]                         │
  │  ◄───────────────────────────────────────────────────┘
  ▼
Firmware gets read value
```

The two sides of the socket speak the *virtmcu wire protocol*, defined in
`hw/rust/common/virtmcu-api/src/core.fbs`. That FlatBuffers schema is the **single source of truth** for
both the QEMU device and the SystemC adapter — never duplicate or manually pack the structs.

---

## The Wire Protocol (`core.fbs`)

All connections must begin with a **handshake**:
```c
struct virtmcu_handshake { /* 8 bytes */
    uint32_t magic;        /* 0x564D4355 ("VMCU") */
    uint32_t version;      /* 1 */
};
```

### Request Payload

```c
struct mmio_req {          /* QEMU → adapter, 32 bytes */
    uint8_t  type;         /* MMIO_REQ_READ (0) or MMIO_REQ_WRITE (1) */
    uint8_t  size;         /* access width: 1, 2, 4, or 8 bytes */
    uint16_t reserved1;
    uint32_t reserved2;
    uint64_t vtime_ns;     /* QEMU virtual time in nanoseconds */
    uint64_t addr;         /* byte offset within the mapped region */
    uint64_t data;         /* write payload (ignored for reads) */
};
```

### Response Payload

```c
struct sysc_msg {          /* adapter → QEMU, 16 bytes */
    uint32_t type;         /* SYSC_MSG_RESP (0), SYSC_MSG_IRQ_SET (1), SYSC_MSG_IRQ_CLEAR (2) */
    uint32_t irq_num;      /* IRQ index (ignored for RESP) */
    uint64_t data;         /* read value (ignored for writes and IRQs) */
};
```

The protocol is synchronous for MMIO: QEMU sends one request and *blocks* waiting for
exactly one response before returning to the firmware.

---

## The QEMU side — `mmio-socket-bridge` QOM device (`hw/rust/backbone/mmio-socket-bridge/src/lib.rs`)

The device registers a `MemoryRegion` of a configurable size. Every firmware
`ld`/`str` to that region invokes `bridge_read()` or `bridge_write()`, which
serialise a `MmioReq` and wait for a `SyscMsg`.

### Big QEMU Lock (BQL) discipline via Inversion of Control

QEMU's vCPU thread holds the BQL while executing translated code. Any
blocking syscall while holding it deadlocks the main loop (QMP, GDB, I/O
would all freeze). 

Instead of writing error-prone manual lock/unlock sequences (which are highly susceptible to Lock-Order Inversion), VirtMCU uses the `virtmcu_qom::cosim::CoSimBridge` IoC framework.

The vCPU thread simply calls:

```rust
let response = self.shared.send_req_and_wait(req);
```

The framework automatically:
1. Registers the vCPU in an RAII `VcpuDrain` tracker (ensuring it cannot be Use-After-Free'd during teardown).
2. Sends the request to the background thread that manages the Unix socket.
3. Uses `virtmcu_qom::sync::Condvar::wait_yielding_bql()` to safely yield the BQL, sleep the vCPU, and re-acquire the BQL when the background thread receives the response.

**Never** put a blocking syscall inside the BQL window manually. Always use the `CoSimBridge` abstraction.

### Device properties

| Property | Type | Default | Description |
|---|---|---|---|
| `socket-path` | string | (required) | Path to the Unix socket |
| `region-size` | uint32 | 0x1000 | Size of the MMIO window in bytes |
| `base-addr` | uint64 | (unmapped) | Map the region at this guest PA at realize |

Example QEMU CLI:
```
-device mmio-socket-bridge,socket-path=/tmp/sc.sock,region-size=4096,base-addr=0x50000000
```

---

## SystemC side — `tools/systemc_adapter/main.cpp`

The adapter has two SystemC modules:

1. **`RegisterFile`** — a `simple_target_socket` that stores 256 32-bit
   registers.  `b_transport()` dispatches reads and writes.

2. **`QemuAdapter`** — an `SC_THREAD` that listens on the Unix socket, deserialises
   `mmio_req` messages into TLM-2.0 `tlm_generic_payload` objects, calls
   `b_transport()` on the downstream socket, and sends back the `mmio_resp`.

### Known limitation — SC_THREAD blocking

`QemuAdapter::run()` makes raw blocking POSIX calls (`accept`, `read`) from
inside an `SC_THREAD`.  SystemC coroutines share one OS thread; a blocking
syscall freezes every other `SC_PROCESS` until the call returns.  This is
acceptable here because `RegisterFile` is purely reactive and does nothing
outside `b_transport()`.

If you extend this to a **multi-module** simulation (e.g., a sensor model that
also runs a periodic `SC_THREAD`), you **must** move the socket server to a
`std::thread` and use an `sc_event` to hand work back to the SystemC scheduler.

---

## Hands-on: running the demo

```bash
# 1. Build the adapter
make -C tools/systemc_adapter

# 2. In terminal A — start the adapter
tools/systemc_adapter/build/adapter /tmp/sc.sock

# 3. In terminal B — start QEMU with a test firmware
#    (see tests/fixtures/guest_apps/irq_stress/smoke_test.sh for a complete example)
scripts/run.sh --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    --kernel /tmp/irq_stress_fw.elf \
    -device mmio-socket-bridge,socket-path=/tmp/sc.sock,region-size=4096,base-addr=0x50000000 \
    -nographic

# Adapter output should show:
# [SystemC] Wrote deadbeef to reg 0
# [SystemC] Read deadbeef from reg 0
```

---

## Exercise: extend the register file

1. Modify `RegisterFile::b_transport()` to make register 1 read-only (always
   returns `0xbaadcafe`).
2. Write firmware that writes to register 1, then reads it back and branches to
   a "pass" or "fail" label depending on whether the read value equals the
   write value.
3. Verify with `get_pc()` in pytest that the firmware ends up in the expected
   branch.

## What's next

- **Path B** (Remote Port, task 5.2): full TLM-2.0 co-simulation for Verilated
  FPGA fabric — higher throughput, more complex setup.
- **Advanced Coordination** (native Zenoh plugin): replaces the Python timing layer with a
  native QOM device that hooks the TCG loop directly.

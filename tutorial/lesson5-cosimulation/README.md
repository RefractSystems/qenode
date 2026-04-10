# Lesson 5: Hardware Co-Simulation — Connecting SystemC Models to QEMU

In this lesson you will learn how QEMU's MMIO subsystem can be extended to
communicate with an external hardware model — specifically a **SystemC TLM-2.0**
register file — over a Unix domain socket.  This is the *Path A* co-simulation
strategy described in `docs/ARCHITECTURE.md §9`.

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
  │  struct mmio_req  [24 bytes]
  │  ─────────────► Unix Domain Socket ─────────────►
  │                                                  SystemC QemuAdapter
  │                                                      │  b_transport()
  │                                                      ▼
  │                                                  RegisterFile
  │                                                      │
  │  struct mmio_resp [ 8 bytes]                         │
  │  ◄───────────────────────────────────────────────────┘
  ▼
Firmware gets read value
```

The two sides of the socket speak the *virtmcu wire protocol*, defined in
`hw/misc/virtmcu_proto.h`.  That header is the **single source of truth** for
both the QEMU device and the SystemC adapter — never duplicate the structs.

---

## The Wire Protocol (`hw/misc/virtmcu_proto.h`)

```c
struct mmio_req {          /* QEMU → adapter, 24 bytes */
    uint8_t  type;         /* MMIO_REQ_READ (0) or MMIO_REQ_WRITE (1) */
    uint8_t  size;         /* access width: 1, 2, 4, or 8 bytes */
    uint16_t reserved1;
    uint32_t reserved2;
    uint64_t addr;         /* byte offset within the mapped region */
    uint64_t data;         /* write payload (ignored for reads) */
};

struct mmio_resp {         /* adapter → QEMU, 8 bytes */
    uint64_t data;         /* read value (ignored for writes) */
};
```

The protocol is synchronous: QEMU sends one request and *blocks* waiting for
exactly one response before returning to the firmware.

---

## QEMU side — `mmio-socket-bridge` QOM device (`hw/misc/mmio-socket-bridge.c`)

The device registers a `MemoryRegion` of a configurable size.  Every firmware
`ld`/`str` to that region invokes `bridge_read()` or `bridge_write()`, which
serialise a `struct mmio_req` and wait for a `struct mmio_resp`.

### Big QEMU Lock (BQL) discipline

QEMU's vCPU thread holds the BQL while executing translated code.  Any
blocking syscall while holding it deadlocks the main loop (QMP, GDB, I/O
would all freeze).  The bridge releases BQL before **both** `write()` and
`read()`:

```c
bql_unlock();
write(sock_fd, &req, sizeof(req));   /* can block if adapter is slow */
read(sock_fd,  &resp, sizeof(resp)); /* blocks until adapter replies  */
bql_lock();
```

**Never** put a blocking syscall inside the BQL window.  This is the same
pattern used by QEMU's NIC backends and block devices.

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
#    (see test/phase5/smoke_test.sh for a complete example)
scripts/run.sh --dtb test/phase1/minimal.dtb \
    --kernel /tmp/phase5_fw.elf \
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
- **Phase 7** (native Zenoh plugin): replaces the Python timing layer with a
  native QOM device that hooks the TCG loop directly.

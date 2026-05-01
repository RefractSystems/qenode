# Lesson 9 — Co-simulating Shared Buses with SystemC

This lesson demonstrates how `virtmcu` can be used to co-simulate a virtual MCU in QEMU alongside a highly accurate, shared physical bus modeled in SystemC. We use a minimal "CAN-lite" implementation as our example.

## Why Co-Simulate?

QEMU is excellent at executing CPU instructions quickly and modeling standard SoC peripherals (UARTs, timers, memory controllers). However, modeling complex physical layers—such as the arbitration steps of a Controller Area Network (CAN) bus, signal integrity, or analog attenuation—is out of scope for a pure CPU emulator.

By bridging QEMU to SystemC:
1. **QEMU** runs the firmware and handles the CPU, memory, and simple MMIO peripherals.
2. **SystemC** models the complex timing, bit-level arbitration, and electrical characteristics of the shared medium.

## Architecture of the Educational CAN Model

Our example consists of three major components:

1. **`mmio-socket-bridge` (QEMU)**: An MMIO peripheral instantiated dynamically via Device Tree. Any firmware reads/writes to its address space are forwarded over a UNIX socket to the SystemC adapter.
2. **`CanController` (SystemC)**: A simple TLM-2.0 target module in `tools/systemc_adapter/main.cpp`. It exposes registers (TX_ID, TX_DATA, CMD, STATUS, RX_ID, RX_DATA) and triggers virtual IRQs back to QEMU when a frame is received.
3. **`SharedMedium` (SystemC + Zenoh)**: A module that simulates the physical CAN bus. When a `CanController` transmits, the `SharedMedium` encapsulates the frame and publishes it to the Zenoh topic `sim/systemc/frame/{node_id}/tx`.

## The Zenoh Coordinator

The `zenoh_coordinator` (developed for coordinator stress and upgraded for FTRT timing) subscribes to `sim/systemc/frame/*/tx`.

When it receives a frame from Node 1:
1. It inspects the `delivery_vtime_ns` timestamp.
2. It adds a propagation/arbitration delay (e.g., 1ms).
3. It forwards the frame to `sim/systemc/frame/Node2/rx`.

When Node 2's `SharedMedium` receives the message, it queues it. The SystemC kernel waits until the virtual time reaches the delivery time, then passes the frame to Node 2's `CanController`, which finally raises an IRQ in Node 2's QEMU instance.

## Execution Flow

1. Firmware on Node 1 writes to the `TX_DATA` and `TX_ID` registers of the bridge.
2. Firmware writes `1` to the `CMD` register.
3. QEMU pauses the vCPU and sends an MMIO `WRITE` request over the UNIX socket to SystemC.
4. SystemC's `CanController` processes the write, constructs a `CanFrame`, and passes it to the `SharedMedium`.
5. `SharedMedium` publishes the frame via Zenoh-C.
6. SystemC sends a response to QEMU, unpausing the vCPU.
7. The Zenoh Coordinator routes the frame to Node 2 with a deterministic virtual time delay.
8. Node 2's SystemC adapter receives the frame via Zenoh.
9. Node 2's `SharedMedium` simulates local bus delay and gives the frame to Node 2's `CanController`.
10. Node 2's `CanController` sends an `IRQ_SET` message over the UNIX socket to Node 2's QEMU.
11. Node 2's QEMU raises the GIC interrupt, and the guest firmware jumps to the ISR to read the `RX_DATA`.

## Key Takeaway

SystemC handles the *bus*, while QEMU handles the *CPU*. Because all communication is stamped with virtual time (`delivery_vtime_ns`), the entire multi-node simulation remains perfectly deterministic and repeatable, regardless of the host OS scheduler or network latency.

---

## Hands-On Walkthrough

### Prerequisites

- QEMU built with the `arm-generic-fdt` patches and `mmio-socket-bridge` module.
  Run `scripts/setup-qemu.sh` if not already done.
- CMake ≥ 3.14, a C++17 compiler, `zenoh-c` built in `third_party/zenoh-c/`.
- `arm-none-eabi-gcc`, `dtc`, and `python3` on `PATH`.
- `eclipse-zenoh` Python package: `uv sync` or `uv pip install eclipse-zenoh`

### Step 1 — Build the SystemC adapter

```bash
cd tools/systemc_adapter
cmake -B build
cmake --build build -j$(nproc)
# Binary: tools/systemc_adapter/build/adapter
```

The build automatically fetches SystemC 3.0.0 via CMake `FetchContent`.

### Step 2 — Write single-node RegisterFile firmware

This minimal firmware writes `1` to register 255 of the bridge (which triggers IRQ 0
in the SystemC adapter), then polls the GIC pending register and prints `REG-OK`.

```asm
/* firmware/reg_irq.S */
.equ UART0_DR,      0x09000000
.equ BRIDGE_BASE,   0x50000000
.equ GICD_ISPENDR1, 0x08000204   /* SPI[0..31] pending; bit 0 = SPI 0 (IRQ 32) */

.global _start
_start:
    /* Write 1 to bridge register 255 → SystemC raises IRQ 0 */
    ldr r0, =BRIDGE_BASE
    add r0, r0, #(255 * 4)
    mov r1, #1
    str r1, [r0]

poll:
    ldr r0, =GICD_ISPENDR1
    ldr r1, [r0]
    tst r1, #1
    beq poll

    /* Print REG-OK */
    ldr r0, =UART0_DR
    adr r1, msg
    ldmia r1, {r2-r7}
    str r2, [r0]
    str r3, [r0]
    str r4, [r0]
    str r5, [r0]
    str r6, [r0]
    str r7, [r0]
1:  wfi
    b 1b

msg:  .byte 'R','E','G','-','O','K',0,0
```

```bash
arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib \
    -T tests/fixtures/guest_apps/ftrt_timing/link.ld firmware/reg_irq.S -o /tmp/reg_irq.elf
```

### Step 3 — Create the Device Tree

The `mmio-socket-bridge` at `0x50000000` connects to the adapter over a UNIX socket.
One GIC SPI interrupt is wired from the bridge to the CPU:

```dts
bridge: bridge@50000000 {
    compatible = "mmio-socket-bridge";
    reg = <0x0 0x50000000 0x0 0x1000>;
    socket-path = "/tmp/test.sock";
    region-size = <0x1000>;
    interrupt-parent = <&gic>;
    interrupts = <0 0 4>;   /* SPI 0, level-high → IRQ 32 */
};
```

See `tests/fixtures/guest_apps/ftrt_timing/smoke_test.sh` for the full DTS that also adds a CPU, RAM, GIC, and
UART — or run the smoke test directly (Step 5 below).

### Step 4 — Run the single-node scenario manually

Open three terminals:

**Terminal 1 — Start the SystemC adapter (standalone/RegisterFile mode):**
```bash
tools/systemc_adapter/build/adapter /tmp/test.sock
# Prints: [SystemC] Listening on /tmp/test.sock...
```

**Terminal 2 — Start QEMU once the socket appears:**
```bash
# Wait for socket, then:
scripts/run.sh --dtb /tmp/test.dtb --kernel /tmp/reg_irq.elf \
    -nographic -monitor none \
    -icount shift=0,align=off,sleep=off
```

**Expected UART output (Terminal 2):**
```
REG-OK
```

**Expected adapter output (Terminal 1):**
```
[SystemC] QEMU connected.
[SystemC] Wrote 1 to reg 255 (addr 1020)
```

### Step 5 — Run the automated smoke tests

The FTRT timing smoke test exercises both scenarios (RegisterFile IRQ and CAN Zenoh RX):

```bash
bash tests/fixtures/guest_apps/ftrt_timing/smoke_test.sh
```

Expected output:
```
[ftrt_timing] Building SystemC adapter...
[ftrt_timing] --- TEST 1: RegisterFile MMIO + IRQ ---
[ftrt_timing] Starting SystemC adapter (standalone)...
[ftrt_timing] Waiting for TEST 1 results...
[ftrt_timing] TEST 1 SUCCESS!
[ftrt_timing] --- TEST 2: CAN Zenoh RX → IRQ ---
[ftrt_timing] Starting SystemC adapter (node=p9-test)...
[ftrt_timing] Injecting Zenoh CAN frame...
[ftrt_timing] Waiting for TEST 2 results...
[ftrt_timing] TEST 2 SUCCESS!
```

### Step 6 — Explore the two-node CAN scenario

When the adapter is started with a `node_id` argument it instantiates a `CanController`
and `SharedMedium` instead of the plain `RegisterFile`:

```bash
# Node 1 (transmitter)
tools/systemc_adapter/build/adapter /tmp/node1.sock node1 &

# Node 2 (receiver)
tools/systemc_adapter/build/adapter /tmp/node2.sock node2 &
```

Node 1 firmware workflow (register map for `CanController` at `BRIDGE_BASE`):

| Offset | Register | R/W | Description |
|--------|----------|-----|-------------|
| 0x00 | TX_ID | W | CAN frame ID to transmit |
| 0x04 | TX_DATA | W | CAN frame payload |
| 0x08 | CMD | W | Write `1` to transmit |
| 0x0C | STATUS | R | Bit 0: RX pending, Bit 1: TX ready |
| 0x10 | RX_ID | R | Received CAN frame ID |
| 0x14 | RX_DATA | R | Received CAN frame payload |
| 0x18 | IRQ_CLR | W | Write any value to clear the RX IRQ |

**Minimal Node 1 firmware snippet (ARM assembly):**
```asm
ldr r0, =BRIDGE_BASE
ldr r1, =0x42        /* TX_ID = 0x42 */
str r1, [r0, #0x00]
ldr r1, =0xDEADBEEF  /* TX_DATA */
str r1, [r0, #0x04]
mov r1, #1           /* CMD: transmit */
str r1, [r0, #0x08]
```

The `SharedMedium` publishes the frame to `sim/systemc/frame/node1/tx`.
The `zenoh_coordinator` (started separately) routes it to `sim/systemc/frame/node2/rx`
with a 1 ms propagation delay added to `delivery_vtime_ns`.

**Starting the coordinator:**
```bash
# From tools/zenoh_coordinator/
cargo run --release
```

**Injecting a frame directly for testing (no Node 1 QEMU needed):**
```python
import zenoh

s = zenoh.open(zenoh.Config())
pub = s.declare_publisher("sim/systemc/frame/node2/rx")

# CanWireFrame: delivery_vtime_ns(8) + size(4) + can_id(4) + can_data(4) = 20 bytes
vtime = 1_000_000
sz = 8
can_id = 0x42
can_data = 0xDEADBEEF
frame = vtime.to_bytes(8, 'little') + sz.to_bytes(4, 'little') + can_id.to_bytes(4, 'little') + can_data.to_bytes(4, 'little')
pub.put(frame)
s.close()
```

### Debugging Tips

**IRQ not firing?**
- Check `GICD_ISPENDR1` (offset 0x204 from `GICD_BASE`). Bit N = SPI N pending.
- The DTS `interrupts = <0 0 4>` maps to `s->irqs[0]` in the bridge → IRQ 32 in the GIC.
- Verify the adapter's `trigger_irq(0, true)` log line: `[SystemC] Wrote 1 to reg 255`.

**MMIO read returns 0?**
- Ensure the adapter is running *before* QEMU connects (socket must exist).
- Add `-d guest_errors` to QEMU to surface MMIO address errors.

**CAN frame not delivered?**
- Check that `delivery_vtime_ns` is non-zero in the published frame.
- The `process_rx` SC_THREAD only delivers after `sc_time_stamp()` reaches the delivery
  time. If the adapter's SystemC clock is behind, the frame queues silently.
- Inspect with: `zenoh_coordinator` logs show routed frames with their vtime stamps.

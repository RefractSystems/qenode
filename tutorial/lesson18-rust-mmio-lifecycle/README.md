# Lesson 18: The MMIO Lifecycle — From Firmware to Physics via Rust and Zenoh

**Objective**: Understand the exact path a byte of data takes when a firmware program writes to a hardware register, and how that translates to a physical action in a deterministic digital twin.

---

## Introduction: The Great Illusion

When you write firmware for a microcontroller, your code lives in a world of absolute physical certainty. If you write the value `0x7F` to memory address `0x40013000`, you expect a physical pin on the chip to start generating a Pulse Width Modulation (PWM) signal. You expect a drone motor to spin up.

In a cyber-physical simulator like **virtmcu**, that physical chip doesn't exist. Instead, we have to construct an elaborate, lightning-fast illusion. We must trick the firmware into thinking it is talking to silicon, while actually routing its commands across process boundaries, programming languages, and network sockets—all without losing track of a single virtual nanosecond.

This tutorial is the story of a single Memory-Mapped I/O (MMIO) write. We will follow one instruction on its journey from the guest firmware, through the QEMU emulator, across the Rust FFI boundary, and finally out to a physics engine.

---

## Act I: The Guest Instruction (Firmware & TCG)

Our story begins inside the guest firmware (compiled for an ARM Cortex-M4). The firmware wants to set a motor's duty cycle to 50% (`0x7F`). It executes a standard store instruction:

```assembly
LDR R0, =0x40013000  // Load the base physical address of the PWM peripheral
LDR R1, =0x0000007F  // Load the target 50% duty cycle value
STR R1, [R0, #0x04]  // Store the value to the PWM_DUTY register (offset 0x04)
```

The firmware doesn't know it's running inside QEMU. It just asks the CPU to write to memory.

However, QEMU's **Tiny Code Generator (TCG)** is watching. As TCG translates this ARM assembly into host x86 or ARM64 instructions, it realizes that `0x40013000` is not standard RAM. It is mapped as an **MMIO region**.

Instead of writing to a host RAM buffer, QEMU's software memory management unit (`softmmu`) traps the execution.

---

## Act II: The Routing (QOM & MemoryRegions)

Once QEMU traps the memory access, it needs to figure out *what* lives at address `0x40013000`.

During the virtual machine's boot process, our platform description file (YAML or Device Tree) told QEMU to instantiate a custom peripheral device and map it to that exact address. QEMU traverses its internal memory tree and locates the `MemoryRegionOps` C struct associated with our device.

This struct contains function pointers for handling reads and writes. QEMU prepares to call the `write` callback.

**Crucially**, QEMU subtracts the base address before making the call. It passes the **relative offset** (`0x04`) and the data (`0x7F`) to the callback. The peripheral model never needs to know where it is mapped in the global address space.

---

## Act III: The Language Boundary (Rust FFI & The BQL)

In legacy QEMU, the `write` callback would just be a C function that updates a variable. But in virtmcu, our core peripherals are written in **safe Rust**.

We hit the language boundary. QEMU calls an `extern "C"` trampoline function provided by our `virtmcu-qom` library.

```rust
// The Trampoline (simplified)
#[no_mangle]
pub unsafe extern "C" fn my_device_write_trampoline(
    opaque: *mut c_void, 
    offset: hwaddr, 
    value: u64, 
    size: c_uint
) {
    // 1. Safely cast the raw C pointer back into our Rust object
    let device = &mut *(opaque as *mut MyRustPeripheral);
    
    // 2. Call the safe Rust trait method
    device.write(offset, value, size);
}
```

### The Danger Zone: The Big QEMU Lock (BQL)
At this exact moment, the thread executing this code is a QEMU **vCPU thread**. And because it is processing an MMIO instruction, it is holding the **Big QEMU Lock (BQL)**.

If our Rust peripheral needs to do something slow—like waiting for an external SystemC process to respond over a UNIX socket—we **cannot block the thread**. If we block while holding the BQL, the entire QEMU emulator deadlocks. The console will freeze, networking will stop, and the simulation will die.

To survive, the Rust peripheral must safely drop the BQL before waiting, and pick it back up when it's done. We handle this using an elegant RAII pattern in Rust:

```rust
// Inside the Rust peripheral's write method
if needs_to_wait_for_systemc {
    // Safely yield the lock. The BQL is released here.
    let _bql_unlock = Bql::temporary_unlock(); 
    
    // Now it is safe to block on a socket or condition variable!
    wait_for_external_process();
    
    // When `_bql_unlock` goes out of scope, the Drop trait 
    // automatically re-acquires the BQL for us.
}
```

---

## Act IV: The Physical Bridge (Zenoh & SAL/AAL)

Assuming our peripheral doesn't need to wait for SystemC, it simply updates its internal state: setting the virtual duty cycle to `0x7F`.

But we aren't done. The firmware expects a physical motor to spin. Our peripheral is part of the **Sensor/Actuator Abstraction Layer (SAL/AAL)**. It must notify the physics engine (like MuJoCo) about this change.

1.  **Read Virtual Time**: The peripheral asks QEMU for the exact virtual time: "At what exact nanosecond did this instruction execute?" (`qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL)`).
2.  **Serialize**: It packs the virtual timestamp and the new duty cycle into a highly optimized binary payload (e.g., using FlatBuffers).
3.  **Dispatch**: It hands the payload to its internal **Zenoh** publisher.

```rust
// Dispatching the physical action
let payload = pack_duty_cycle(virtual_time, 0x7F);
self.zenoh_publisher.put(payload).wait();
```

The message flies out over the network to the topic `sim/actuator/pwm/0`.

Miles away (or just in another Docker container), the physics engine receives the message. It looks at the virtual timestamp, advances its internal physics step to that exact microsecond, and applies a new physical torque to the 3D model of the drone rotor.

The illusion is complete.

---

## Conclusion

What looked like a simple memory assignment in C (`*pwm_duty = 0x7F;`) triggered a magnificent chain reaction:
1.  **Firmware** executing an ARM instruction.
2.  **QEMU TCG** intercepting a memory trap.
3.  **QOM** routing the offset to a device struct.
4.  **virtmcu-qom** trampling across the FFI boundary into safe Rust.
5.  **Rust** managing the Big QEMU Lock to prevent deadlocks.
6.  **Zenoh** dispatching the timestamped state change across the network to a physics engine.

By handling this entire pipeline natively inside QEMU's address space using Rust, virtmcu achieves near bare-metal emulation speeds while supporting infinitely complex, distributed physical environments.

### Hands-On Exercise
To see this in action, you can use QEMU's built-in tracing.
1. Run a virtmcu simulation with the `-trace "memory_region_ops_*" ` flag.
2. Watch the console as your firmware boots. You will see QEMU log every single time an MMIO write transitions from the emulator into one of our custom peripherals.
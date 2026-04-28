# Lesson 19: Native Rust Migration & Safety

**Objective**: Learn how virtmcu handles the transition through the Rust FFI boundary, manages the Big QEMU Lock (BQL) with RAII patterns, and dispatches deterministic payloads over Zenoh to the physics engine.

---

## Introduction

Continuing from Lesson 18, QEMU has trapped a memory write to `0x40013000` with offset `0x04` and value `0x7F`. QEMU now calls the `write` callback for the peripheral. In virtmcu, our core peripherals are written in **safe Rust**. We now cross the language boundary.

---

## Act I: The Language Boundary (Rust FFI & The BQL)

QEMU calls an `extern "C"` trampoline function provided by our `virtmcu-qom` library.

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

## Act II: The Physical Bridge (Zenoh & SAL/AAL)

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

The MMIO pipeline seamlessly bridges two worlds:
1.  **virtmcu-qom** trampling across the FFI boundary into safe Rust.
2.  **Rust** managing the Big QEMU Lock to prevent deadlocks.
3.  **Zenoh** dispatching the timestamped state change across the network to a physics engine.

By handling this entire pipeline natively inside QEMU's address space using Rust, virtmcu achieves near bare-metal emulation speeds while supporting infinitely complex, distributed physical environments.
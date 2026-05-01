# Lesson 18: The MMIO Lifecycle

**Objective**: Understand the exact path a byte of data takes when a firmware program writes to a hardware register, and how that translates to a physical action in a deterministic digital twin.

---

## Introduction: The Great Illusion

When you write firmware for a microcontroller, your code lives in a world of absolute physical certainty. If you write the value `0x7F` to memory address `0x40013000`, you expect a physical pin on the chip to start generating a Pulse Width Modulation (PWM) signal. You expect a drone motor to spin up.

In a cyber-physical simulator like **virtmcu**, that physical chip doesn't exist. Instead, we have to construct an elaborate, lightning-fast illusion. We must trick the firmware into thinking it is talking to silicon.

This tutorial is the story of a single Memory-Mapped I/O (MMIO) write. We will follow one instruction on its journey from the guest firmware through the QEMU emulator.

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

## Conclusion

What looked like a simple memory assignment in C (`*pwm_duty = 0x7F;`) triggered a magnificent chain reaction:
1.  **Firmware** executing an ARM instruction.
2.  **QEMU TCG** intercepting a memory trap.
3.  **QOM** routing the offset to a device struct.

In the next lesson (Lesson 19), we will see what happens when this call crosses the language boundary into Rust.

### Hands-On Exercise
To see this in action, you can use QEMU's built-in tracing.
1. Run a virtmcu simulation with the `-trace "memory_region_ops_*" ` flag.
2. Watch the console as your firmware boots. You will see QEMU log every single time an MMIO write transitions from the emulator into one of our custom peripherals.
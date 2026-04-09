# Lesson 1: Dynamic Machines, Device Trees, and Bare-Metal Debugging

Welcome to Lesson 1! By the end of this tutorial, you will have constructed a virtual ARM hardware board from a text file, written bare-metal assembly to interact with a serial port, and debugged the emulation at the instruction level.

## Terminology Check

*   **QEMU**: A fast, cross-platform hardware emulator.
*   **Renode**: A framework known for dynamic hardware composition (building boards at runtime).
*   **qenode**: Our hybrid framework that patches QEMU to get Renode's dynamic features while keeping QEMU's speed.
*   **MMIO (Memory-Mapped I/O)**: Hardware peripherals (like sensors or serial ports) are mapped into the CPU's memory address space. To send a character to a serial port, the CPU literally writes bytes to a specific memory address.
*   **Device Tree (DTS/DTB)**: A standardized data structure used to describe hardware to an operating system (or, in our case, to QEMU). A `.dts` file is human-readable text; a `.dtb` is the compiled binary version.
*   **Bare-metal**: Running code directly on the hardware (or emulator) without an Operating System. You have no `printf`, no threads, and no file system!

## Part 1: Defining the Hardware (The Device Tree)

Usually, QEMU defines its hardware boards (like the Raspberry Pi) inside compiled C code. 
In qenode, we use the `arm-generic-fdt` machine type. This special QEMU machine boots up completely empty and reads a Device Tree Blob (DTB) to figure out what CPUs, memory, and peripherals it should instantiate.

Open the file `src/minimal.dts`.

```dts
// ...
    cpus {
        cpu@0 {
            device_type = "cpu";
            compatible = "cortex-a15-arm-cpu";
            reg = <0>;
        };
    };
// ...
```

### 🧠 Under the Hood: QOM Types
Notice the `compatible` string. In standard Linux device trees, this might look like `"arm,cortex-a15"`. However, QEMU organizes everything via the **QEMU Object Model (QOM)**. Our patched machine directly passes this compatible string to `object_new()` internally. Therefore, we must use the exact QOM type name: `"cortex-a15-arm-cpu"`.

## Part 2: Writing Bare-Metal Software

We have a CPU and a UART (Serial Port). Now we need software.
Since we don't have an OS, we'll write ARM assembly. Open `src/hello.S`.

```assembly
_start:
    ldr r0, =0x09000000    // Load the UART memory address
    mov r1, #'H'           // Prepare the character 'H'
    str r1, [r0]           // Write it to memory (triggers the UART hardware!)
```

This code loads the physical address of the PL011 UART (`0x09000000`)—which we defined in our DTS file!—and writes the letters "HI\n" to it.

To compile this, we need a **Linker Script** (`src/linker.ld`). A linker script tells the compiler *where* the code should live in physical memory. We set it to `0x40000000`, matching our `memory` node from the DTS.

## Part 3: Compiling and Running

Let's compile the hardware description (DTS) and the software (Assembly).

1. Open a terminal in the `tutorial/lesson1-dynamic-machines` folder.
2. Run `make`.

This produces `minimal.dtb` and `hello.elf`.
Now, let's run it using the `qenode` runner script!

```bash
../../scripts/run.sh --machine arm-generic-fdt --dtb minimal.dtb --kernel hello.elf -nographic
```

You should see it print `HI` to your terminal, and then hang (because we put the CPU to sleep with the `wfi` instruction).
Press `Ctrl+A`, then `X` to exit QEMU.

## Part 4: Debugging with GDB

As a graduate student/researcher, you will inevitably write code that crashes or doesn't behave as expected. Knowing how to peek inside the CPU is a superpower.

We can tell QEMU to pause at startup and listen for a debugger. Run this command:

```bash
../../scripts/run.sh --machine arm-generic-fdt --dtb minimal.dtb --kernel hello.elf -nographic -s -S
```
*(The `-s` flag opens a GDB server on port 1234. The `-S` flag tells the CPU not to start executing until instructed).*

QEMU will appear to hang. Open a **second terminal** and run the ARM debugger:

```bash
gdb-multiarch hello.elf
```

Inside the `(gdb)` prompt, type the following commands:

1. `target remote :1234` (Connect to QEMU)
2. `layout asm` (Show the assembly instructions being executed)
3. `layout regs` (Show the CPU registers in real-time)
4. `stepi` (Step exactly one assembly instruction)

Press `Enter` a few times to repeat the `stepi` command. Watch as `r0` gets populated with `0x9000000` and `r1` gets the hex value for 'H' (`0x48`). 
When you step over the `str r1, [r0]` instruction, switch back to your first terminal: you'll see the 'H' magically appear!

## Exercises to Try

1. **Modify the Assembly**: Open `src/hello.S` and make it print your name. Run `make` and execute QEMU again.
2. **Break the Linker**: In `src/linker.ld`, change `. = 0x40000000;` to `. = 0x50000000;`. Run `make` and execute QEMU. What happens? (Hint: The CPU will try to execute code from a memory address that doesn't exist, causing a fault!)
3. **Inspect Memory**: While attached to GDB, type `x/4x 0x40000000` to e**X**amine 4 he**X** words of memory starting at the RAM base address. You are looking at the raw compiled instructions of your program!

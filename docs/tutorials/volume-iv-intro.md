# Volume IV: Practical Engineering & Laboratory Mastery

## Translating Theory into Execution

Welcome to the **VirtMCU Laboratory**. In the previous volumes, we established the theoretical foundations and architectural blueprints of our deterministic simulation universe. Now, we transition from theory to practice.

Volume IV is a series of rigorous laboratory exercises designed to build your proficiency in the VirtMCU toolchain. You will learn to construct dynamic machines, write high-performance peripheral plugins in Rust, and automate the complex orchestration required for multi-node simulation.

---

## Laboratory Curriculum

### [The VirtMCU Laboratory (Tutorials)](README.md)

### [Lesson 1: Dynamic Machines](lesson1-dynamic-machines/README.md)
Learn to define and boot a custom ARM machine using only a Device Tree and a YAML topology file.

### [Lesson 2: Dynamic QOM Plugins](lesson2-dynamic-plugins/README.md)
The gateway to extensibility. Master the creation of dynamic shared objects (DSOs) that can be loaded into QEMU at runtime.

### [Lesson 3: Parsing .repl to DTB](lesson3-repl2qemu/README.md)
Bridging ecosystem formats. Learn how to convert Antmicro Renode (`.repl`) files into the Flattened Device Trees (`.dtb`) used by VirtMCU.

### [Lesson 4: Emulation Automation](lesson4-emulation-automation/README.md)
Orchestration at scale. Use Python and `pytest` to automate the execution, monitoring, and verification of your firmware.

### [Lesson 18: MMIO Lifecycle](lesson18-mmio-lifecycle/README.md)
Follow the journey of a single memory access from the guest CPU, through the TCG, and into your custom Rust peripheral model.

### [Lesson 19: Native Rust Migration](lesson19-native-rust-migration/README.md)
The SOTA standard. Learn the patterns and best practices for migrating legacy C peripheral models into modern, memory-safe Rust.

---

## Laboratory Mandates

### 1. The Beyoncé Rule
"If you liked it, then you shoulda put a test on it." In this laboratory, no feature is complete until it is verified by an automated integration test.

### 2. Environment Agnosticism
Your simulations must run identically in your local devcontainer and in the remote CI pipeline. We do not tolerate "it works on my machine."

### 3. Incremental Complexity
We begin with single-node logic and move toward complex, multi-threaded interactions. Do not skip the foundational lessons; they provide the scaffolding for the distributed systems mastery in Volume V.

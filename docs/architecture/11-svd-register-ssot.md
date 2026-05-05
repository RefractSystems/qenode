# Chapter 11: Silicon Definition (SVD) and Register SSOT

## 1. Introduction to CMSIS-SVD in VirtMCU

VirtMCU strives for absolute binary fidelity. Firmware running in the digital twin must execute unmodified. This requires that the virtual peripherals exposed by the QEMU plugin layer precisely match the memory layout (MMIO) of the physical silicon.

To guarantee this, VirtMCU adopts the **CMSIS-SVD (System View Description)** standard as the **Single Source of Truth (SSOT)** for all MMIO definitions. 

SVD is an XML-based industry standard created by ARM. It exhaustively defines the memory map, peripherals, registers, and bitfields of a microcontroller. By leveraging SVD, VirtMCU eliminates the drift that typically occurs when C headers, UI schemas, and emulator backends are written independently.

## 2. Data Flow and Generation Pipeline

The architecture is strictly unidirectional: SVD → Artifacts.

1. **The Source (`.svd` file)**: The SVD XML file resides in the project definitions (e.g., `hw/defs/robot_arm.svd`). It defines peripherals, their base addresses, registers, and offsets.
2. **The Generators (`tools/svd2virtmcu/`)**: Python tools using `cmsis-svd` and `Jinja2` parse the SVD.
3. **The Outputs**:
   - **Rust Constants (`svd_constants.rs`)**: A `build.rs` script in the QEMU plugin or sidecar parses the SVD at compile time. This ensures the Rust backend statically knows the correct offsets and boundaries for packing FlatBuffers or executing logic.
   - **C Headers (`robot_io.h`)**: `svd2header.py` generates the bare-metal C headers used by the firmware. It embeds `_Static_assert` validations to enforce alignment and type sizes at the compiler level.
   - **UI Schemas (`schema.json`)**: `svd2schema.py` infers control boundaries (Targets) and telemetry readouts (State) directly from the SVD, mapping them into the JSON schema required by the React dashboard.

## 3. Why CMSIS-SVD instead of Zephyr/DeviceTree?

Zephyr OS utilizes a highly successful paradigm based on DeviceTree (DTS) and YAML bindings to generate C macros at compile time. While powerful, VirtMCU deliberately chooses SVD over the Zephyr approach for several reasons:

* **RTOS Agnosticism:** Zephyr's macro generation is deeply coupled to its own build system (CMake/Kconfig) and assumes Zephyr's driver model. VirtMCU supports bare-metal C, FreeRTOS, Linux, and Zephyr. SVD provides an RTOS-agnostic, raw silicon description.
* **Industry Standard Vendor Support:** Silicon vendors natively publish SVDs for their chips (e.g., STMicro, NXP, Nordic). By using SVD, we can ingest vendor definitions directly without needing to translate them into custom YAML bindings.
* **Micro-Architecture Focus:** DeviceTree excels at describing board-level topology (e.g., "UART2 is connected to GPIO pins 4 and 5"). SVD excels at micro-architecture (e.g., "The UART2 Baud Rate register is at offset 0x0C and bits 4:7 control parity"). VirtMCU uses both: DeviceTree for the `arm-generic-fdt` QEMU machine layout, and SVD for internal peripheral registers.

## 4. Mapping SVD under the OpenUSD Umbrella

VirtMCU's parent platforms (like FirmwareStudio) are adopting OpenUSD to describe the 3D, physical, and cyber-spatial world. 

**How they interact:**
OpenUSD describes the **Macro-Architecture**; SVD describes the **Micro-Architecture**.

1. **OpenUSD (World Definition):** Defines that a "Robot Arm" exists at coordinates `[0, 0, 0]`, has 3 joints, and mass properties. It defines the macroscopic topology of the digital twin.
2. **The Bridging Link:** The OpenUSD schema contains a custom property (e.g., `reflow:svdPath = "hw/defs/robot_arm.svd"`) attached to the Robot's Cyber-Node prim.
3. **SVD (Silicon Definition):** Defines exactly how the firmware talks to those 3 joints via MMIO. OpenUSD knows *what* the robot is; SVD knows *how the CPU commands it*.

By keeping SVD nested under the OpenUSD node, we cleanly separate 3D/Kinematic modeling from CPU/Register-level modeling while keeping both completely declarative.
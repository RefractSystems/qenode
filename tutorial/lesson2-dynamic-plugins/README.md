# Lesson 2: Dynamic QOM Plugins

Welcome to Lesson 2! In the previous lesson, we learned how to build a machine dynamically using a Device Tree. Now, we will explore how to add entirely new peripheral devices to QEMU *without* modifying the core emulator source code.

## The Problem with Upstream QEMU
In traditional QEMU development, adding a new peripheral (like a custom sensor or an accelerometer) requires modifying QEMU's C source tree, writing the hardware logic, editing Makefiles, and recompiling the entire emulator (a 5–10 minute process).

For research and firmware testing, this tightly coupled approach is tedious.

## The virtmcu Solution: Dynamic Plugins
QEMU has an obscure feature: **modules**. However, it is primarily used for UI components (like GTK or SDL) and audio backends. 
In virtmcu, we exploit this feature to compile our custom peripherals as standalone shared libraries (`.so` on Linux).

We place our C code in the `hw/` directory of the `virtmcu` repository. A symlink bridges this folder into QEMU's build system. When we run `make build`, QEMU automatically compiles our devices into `.so` files.

### 🧠 Under the Hood: The QEMU Object Model (QOM)
To ensure QEMU can dynamically load and instantiate our device, we use the **QEMU Object Model (QOM)**.

Open `hw/dummy/dummy.c`. This is a minimal template for a new peripheral:

1.  **Type Registration**: We define `TYPE_DUMMY_DEVICE "dummy-device"`.
2.  **State Struct**: We define `DummyDeviceState` which inherits from `SysBusDevice`.
3.  **Initialization**: The `dummy_init` function allocates the `MemoryRegion` (the MMIO registers) and binds them to read/write callbacks.
4.  **Module Macro**: The critical line at the bottom is `module_obj(TYPE_DUMMY_DEVICE);`. This tells QEMU's build system to export metadata stating that this `.so` file provides the `dummy-device` object.

When we run QEMU and pass `-device dummy-device`, QEMU's object system notices that `dummy-device` isn't compiled into the main executable, searches its `lib/qemu` directory, finds our `.so`, dynamically loads it via `dlopen()`, and instantiates the object!

## Part 1: Building the Plugin

If you haven't recently, run `make build` from the root of the virtmcu repository.

```bash
make build
```

Behind the scenes, QEMU's `meson` build system sees `hw/dummy/dummy.c` (via the symlink in `third_party/qemu/hw/virtmcu`), recognizes it as a module, and produces `hw-virtmcu-dummy.so`.

## Part 2: Loading the Plugin dynamically

Let's test our new peripheral. We will use the `run.sh` script, which automatically sets the `QEMU_MODULE_DIR` environment variable to ensure QEMU searches our local build folder for `.so` files.

We will boot the empty `arm-generic-fdt` machine and plug our device into it via the command line:

```bash
../../scripts/run.sh --dtb ../../test/phase1/minimal.dtb -device dummy-device -nographic
```

*Note: Since we are not passing a kernel, the CPU will likely fault immediately after boot because there is no code to execute, but the emulator will successfully load the module!*

You can verify it loaded by pressing `Ctrl+A` then `C` to enter the QEMU monitor.

Type the following command to inspect the QOM tree:
```
(qemu) info qom-tree
```

Look closely at the output. Under `/machine/peripheral-anon`, you should see a `device[0] (dummy-device)`! This proves that our out-of-tree shared library was successfully loaded and instantiated at runtime.

## Summary
You have successfully loaded a custom hardware peripheral into QEMU dynamically.
This decoupled architecture allows you to iterate rapidly on hardware models (e.g., sensors, accelerators) by modifying a single C file and doing a fast incremental rebuild, keeping the core emulator pristine.

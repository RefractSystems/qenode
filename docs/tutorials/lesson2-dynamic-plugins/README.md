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
../../scripts/run.sh --dtb ../../tests/fixtures/guest_apps/boot_arm/minimal.dtb -device dummy-device -nographic
```

*Note: Since we are not passing a kernel, the CPU will likely fault immediately after boot because there is no code to execute, but the emulator will successfully load the module!*

You can verify it loaded by pressing `Ctrl+A` then `C` to enter the QEMU monitor.

Type the following command to inspect the QOM tree:
```
(qemu) info qom-tree
```

Look closely at the output. Under `/machine/peripheral-anon`, you should see a `device[0] (dummy-device)`! This proves that our out-of-tree shared library was successfully loaded and instantiated at runtime.

## Part 3: The Rust Interop Story (Hybrid C/Rust Plugins)

While C is the native language of QEMU, writing safe and complex peripheral models is often easier in Rust.  `virtmcu` provides a hybrid C/Rust template in `hw/rust-dummy/`.

### Why not QEMU's native Rust support?

QEMU 10+ ships an official Meson+cargo pipeline for writing device models in Rust (`hw/rust/`).  That pipeline compiles Rust into the **monolithic `qemu-system-*` binary** — it cannot produce standalone `.so` modules.  Our template uses a different approach: `rustc` compiles `src/lib.rs` into a `staticlib`, which Meson links into `hw-virtmcu-rust-dummy.so` alongside the C boilerplate.  This works with `--enable-modules` and keeps your code outside the QEMU source tree.

### How the split works

1. **The QOM Boilerplate (C)** — `hw/rust-dummy/rust-dummy.c`
   - Registers the `TypeInfo`, initialises the `MemoryRegion`, and handles QEMU's object lifecycle.
   - Every MMIO access calls `rust_dummy_read()` / `rust_dummy_write()`, forwarding the device's private state pointer (`priv_state`) so Rust can access per-instance data.

2. **The Device Logic (Rust)** — `hw/rust-dummy/src/lib.rs`
   - A `#[no_std]` crate exporting two `extern "C"` functions.
   - Receives `priv_state: *mut c_void` as its first argument.  In this demo it is `NULL` (stateless), but the template doc-comments show how to allocate Rust-owned state and pass it through.

### Build flow

```
rustc --crate-type=staticlib src/lib.rs  →  librust_dummy.a
                                               ↓ linked into
                             hw-virtmcu-rust-dummy.so
```

Meson drives the build via a `custom_target`.  `Cargo.toml` is present for IDE tooling (`rust-analyzer`, `cargo clippy`) but **Meson is authoritative** — running `cargo build` directly does not produce a QEMU-loadable module.

### Internals: the FFI contract

The C side declares:
```c
extern uint64_t rust_dummy_read(void *priv_state, uint64_t addr, uint32_t size);
extern void     rust_dummy_write(void *priv_state, uint64_t addr, uint64_t val, uint32_t size);
```

The Rust side exports matching symbols:
```rust
#[no_mangle]
pub extern "C" fn rust_dummy_read(_priv_state: *mut c_void, addr: u64, _size: u32) -> u64 {
    match addr { 0 => 0xdead_beef, _ => 0 }
}
```

The `priv_state` parameter is the key to stateful devices.  See the doc-comments in `src/lib.rs` for the full extension pattern using `rust_dummy_init()` and `Box::into_raw`.

### Testing the Rust Plugin

```bash
../../scripts/run.sh --dtb ../../tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    -device rust-dummy,base-addr=0x60000000 \
    -nographic
```

Add `-d unimp` to see every MMIO access logged.  Guest reads from `0x60000000` return `0xdeadbeef`.

## Summary

You have successfully loaded custom hardware peripherals into QEMU dynamically, using both pure C and a hybrid C/Rust approach.  This decoupled architecture lets you iterate rapidly on hardware models (sensors, accelerators, custom registers) by editing a single file and doing a fast incremental rebuild, keeping the QEMU emulator core untouched.

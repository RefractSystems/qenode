# rust-dummy — C/Rust hybrid QOM peripheral template

A minimal template demonstrating how to write a QEMU QOM peripheral where the
QEMU boilerplate stays in C and the device logic lives in a `#[no_std]` Rust
crate linked via raw FFI.

## Architecture

```
hw/rust-dummy/
├── rust-dummy.c      QOM type registration, MemoryRegion setup, FFI call sites
└── src/lib.rs        #[no_std] Rust — actual read/write logic, exported as extern "C"
```

QEMU's MemoryRegion callbacks (`bridge_read`, `bridge_write`) are thin C shims
that forward every MMIO access to `rust_dummy_read()` / `rust_dummy_write()`.
Rust sees plain integers; it never touches QEMU internals.

### Why not QEMU's official Rust support?

QEMU 10+ ships an official Meson+cargo integration (`hw/rust/`) for writing
device models in Rust.  That integration compiles Rust code directly into the
monolithic `qemu-system-*` binary.  It does **not** produce loadable `.so`
modules compatible with `--enable-modules`.

This template uses a different strategy: `rustc` compiles `src/lib.rs` into a
`staticlib` (`.a` archive), which Meson links into `hw-virtmcu-rust-dummy.so`
alongside the C boilerplate.  The result is a dynamically-loadable module that
works with `--enable-modules` and keeps the device code outside the QEMU tree.

### Build flow

```
Meson custom_target
  └─ rustc --crate-type=staticlib src/lib.rs → librust_dummy.a
              ↓ linked into
hw-virtmcu-rust-dummy.so  (contains rust-dummy.c objects + librust_dummy.a)
```

Cargo is present for IDE tooling (`rust-analyzer`, `cargo check`/`clippy`).
**Meson is the authoritative build** — do not rely on `cargo build` for the
QEMU integration.  Running `cargo build` standalone works for development/testing
the Rust logic in isolation, but the resulting binary is not loaded by QEMU.

## Usage

```bash
# Load from the QEMU command line (run.sh sets QEMU_MODULE_DIR automatically):
scripts/run.sh --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    -device rust-dummy,base-addr=0x60000000 \
    -nographic
```

Reads from guest address `0x60000000` return `0xdeadbeef` (offset 0) or `0`
(all other offsets).  Enable tracing with `-d unimp` to see every access logged.

## Extending this template

See the doc-comment at the top of `src/lib.rs` and the inline comments in
`rust-dummy.c` for a step-by-step guide to:

1. Adding per-instance Rust state via a `rust_dummy_init()` / `rust_dummy_fini()`
   lifecycle pair stored in `RustDummyState.rust_priv`.
2. Defining `#[repr(C)]` structs that cross the FFI boundary safely.
3. Accessing QEMU device properties from Rust (via the `priv_state` pointer).

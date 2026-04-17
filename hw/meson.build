# virtmcu hardware peripheral modules.
#
# This file lives at hw/meson.build inside the virtmcu repo.
# scripts/setup-qemu.sh creates a symlink:
#   <qemu-src>/hw/virtmcu  →  <virtmcu-repo>/hw
# and appends `subdir('virtmcu')` to <qemu-src>/hw/meson.build.
#
# With --enable-modules each entry in hw_virtmcu_modules compiles to a
# standalone shared library (hw-virtmcu-<name>.so / .dylib) placed in the
# QEMU module dir.  Without modules it is linked statically into qemu-system-*.
#
# Adding a new device:
#   1. Create hw/<name>/<name>.c following the pattern in hw/dummy/dummy.c.
#   2. Add a source_set and register it below.

hw_virtmcu_modules = {}

# Common FFI helpers for Rust-backed modules
virtmcu_rust_ffi_file = files('misc/virtmcu-rust-ffi.c')

# ── dummy ─────────────────────────────────────────────────────────────────────
# Minimal QOM SysBusDevice.  Always included — it is the smoke-test for the
# dynamic loading pipeline and the template for all new devices.
dummy_ss = ss.source_set()
dummy_ss.add(files('dummy/dummy.c'))
hw_virtmcu_modules += {'dummy': dummy_ss}

# ── mmio-socket-bridge ────────────────────────────────────────────────────────
# Path A co-simulation bridge: forwards MMIO reads/writes over a Unix socket to
# an external process (e.g. the SystemC adapter in tools/systemc_adapter/).
# Compiles to hw-virtmcu-mmio-socket-bridge.so.
mmio_bridge_ss = ss.source_set()
mmio_bridge_ss.add(files('misc/mmio-socket-bridge.c'))
hw_virtmcu_modules += {'mmio-socket-bridge': mmio_bridge_ss}

# ── rust-dummy ──────────────────────────────────────────────────────────────────
# Template: QOM device in C + peripheral logic in Rust (raw FFI, no_std).
# Compiles to hw-virtmcu-rust-dummy.so.
rust_dummy_lib = custom_target(
  'rust_dummy_lib',
  input: 'rust/rust-dummy/src/lib.rs',
  output: ['librust_dummy.a', 'rust_dummy_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release -p rust_dummy && cp target/release/librust_dummy.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'librust_dummy.a', meson.current_build_dir() / 'rust_dummy_trigger.c'
  ]
)
rust_dummy_ss = ss.source_set()
rust_dummy_ss.add(files('rust/rust-dummy/rust-dummy.c'))
rust_dummy_ss.add(virtmcu_rust_ffi_file)
rust_dummy_ss.add(rust_dummy_lib[1])
rust_dummy_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / 'librust_dummy.a']
))
hw_virtmcu_modules += {'rust-dummy': rust_dummy_ss}

# ── zenoh-clock ───────────────────────────────────────────────────────────────
# Rust implementation of the Zenoh virtual clock.
# Compiles to hw-virtmcu-zenoh-clock.so.
rust_zenoh_clock_lib = custom_target(
  'rust_zenoh_clock_lib',
  input: 'rust/zenoh-clock/src/lib.rs',
  output: ['libzenoh_clock.a', 'rust_zenoh_clock_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release -p zenoh_clock && cp target/release/libzenoh_clock.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_clock.a', meson.current_build_dir() / 'rust_zenoh_clock_trigger.c'
  ]
)
zenoh_clock_ss = ss.source_set()
zenoh_clock_ss.add(files('zenoh/zenoh-clock.c'))
zenoh_clock_ss.add(virtmcu_rust_ffi_file)
zenoh_clock_ss.add(rust_zenoh_clock_lib[1])
zenoh_clock_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / 'libzenoh_clock.a']
))
hw_virtmcu_modules += {'zenoh-clock': zenoh_clock_ss}

# ── zenoh-netdev ─────────────────────────────────────────────────────────
# Rust implementation of the Zenoh ethernet backend.
rust_zenoh_netdev_lib = custom_target(
  'rust_zenoh_netdev_lib',
  input: 'rust/zenoh-netdev/src/lib.rs',
  output: ['libzenoh_netdev.a', 'rust_zenoh_netdev_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release -p zenoh-netdev && cp target/release/libzenoh_netdev.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_netdev.a', meson.current_build_dir() / 'rust_zenoh_netdev_trigger.c'
  ]
)

# ── zenoh-chardev ────────────────────────────────────────────────────────
# Rust implementation of the Zenoh serial backend.
rust_zenoh_chardev_lib = custom_target(
  'rust_zenoh_chardev_lib',
  input: 'rust/zenoh-chardev/src/lib.rs',
  output: ['libzenoh_chardev.a', 'rust_zenoh_chardev_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release -p zenoh-chardev && cp target/release/libzenoh_chardev.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_chardev.a', meson.current_build_dir() / 'rust_zenoh_chardev_trigger.c'
  ]
)

subdir('zenoh')

# ── remote-port-bridge ────────────────────────────────────────────────────────
# Path B co-simulation bridge: AMD/Xilinx Remote Port over a Unix socket
# Compiles to hw-virtmcu-remote-port-bridge.so.
remote_port_ss = ss.source_set()
remote_port_ss.add(files(
    'remote-port/remote-port-bridge.c',
    'remote-port/remote-port-proto.c'
))
hw_virtmcu_modules += {'remote-port-bridge': remote_port_ss}

# ── Add new devices above this line ──────────────────────────────────────────
modules += {'hw-virtmcu': hw_virtmcu_modules}

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
#
# Build strategy:
#   1. A custom_target invokes rustc directly to compile src/lib.rs into a
#      static archive (librust_dummy.a).  We avoid cargo here so the build is
#      fully driven by Meson without a network fetch.
#   2. A generated trigger file (rust_dummy_trigger.c) is added to the source
#      set.  Meson must compile it before linking the .so, and because it is an
#      output of the custom_target, this establishes a link-time ordering
#      dependency:  custom_target → trigger.c.o → .so link step.
#   3. link_args names the .a archive by its Meson build-dir path so the linker
#      picks it up.  The ordering via (2) ensures it exists before the link runs.
#
# Why not QEMU's official Rust support?
#   QEMU 10+ can build Rust code via meson+cargo, but it compiles into the
#   monolithic qemu-system-* binary — it does not produce loadable .so modules.
#   This custom_target approach works with --enable-modules and keeps peripheral
#   code outside the QEMU source tree.
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
# Adding the trigger file (output[1] of the custom_target) as a source creates
# a compile-time dep on the custom_target, which transitively makes the .so
# link step depend on librust_dummy.a being present before the linker runs.
rust_dummy_ss.add(rust_dummy_lib[1])
rust_dummy_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / 'librust_dummy.a']
))
hw_virtmcu_modules += {'rust-dummy': rust_dummy_ss}

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

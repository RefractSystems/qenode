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
hw_virtmcu_inc = include_directories('.', 'misc')
virtmcu_rust_ffi_file = files('misc/virtmcu-rust-ffi.c')
virtmcu_ffi_dep = declare_dependency(include_directories: hw_virtmcu_inc)

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
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p rust_dummy && cp target/release/librust_dummy.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'librust_dummy.a', meson.current_build_dir() / 'rust_dummy_trigger.c'
  ]
)
rust_dummy_ss = ss.source_set()
rust_dummy_ss.add(files('rust/rust-dummy/rust-dummy.c'))
rust_dummy_ss.add(virtmcu_rust_ffi_file, virtmcu_ffi_dep)
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
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-clock && cp target/release/libzenoh_clock.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("zenoh-clock");
EOF
''',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_clock.a', meson.current_build_dir() / 'rust_zenoh_clock_trigger.c'
  ]
)
zenoh_clock_ss = ss.source_set()
zenoh_clock_ss.add(virtmcu_ffi_dep)
zenoh_clock_ss.add(virtmcu_rust_ffi_file)
zenoh_clock_ss.add(rust_zenoh_clock_lib[1])
zenoh_clock_ss.add(rust_zenoh_clock_lib[0])
zenoh_clock_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libzenoh_clock.a', '-Wl,--no-whole-archive']
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
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-netdev && cp target/release/libzenoh_netdev.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
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
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-chardev && cp target/release/libzenoh_chardev.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_chardev.a', meson.current_build_dir() / 'rust_zenoh_chardev_trigger.c'
  ]
)

# ── zenoh-telemetry ────────────────────────────────────────────────────────
# Rust implementation of the Zenoh telemetry backend.
rust_zenoh_telemetry_lib = custom_target(
  'rust_zenoh_telemetry_lib',
  input: 'rust/zenoh-telemetry/src/lib.rs',
  output: ['libzenoh_telemetry.a', 'rust_zenoh_telemetry_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-telemetry && cp target/release/libzenoh_telemetry.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_telemetry.a', meson.current_build_dir() / 'rust_zenoh_telemetry_trigger.c'
  ]
)

# ── zenoh-actuator ────────────────────────────────────────────────────────
# Rust implementation of the Zenoh actuator backend.
rust_zenoh_actuator_lib = custom_target(
  'rust_zenoh_actuator_lib',
  input: 'rust/zenoh-actuator/src/lib.rs',
  output: ['libzenoh_actuator.a', 'rust_zenoh_actuator_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-actuator && cp target/release/libzenoh_actuator.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_actuator.a', meson.current_build_dir() / 'rust_zenoh_actuator_trigger.c'
  ]
)

# ── zenoh-802154 ─────────────────────────────────────────────────────────
# Rust implementation of the Zenoh 802.15.4 radio.
rust_zenoh_802154_lib = custom_target(
  'rust_zenoh_802154_lib',
  input: 'rust/zenoh-802154/src/lib.rs',
  output: ['libzenoh_802154.a', 'rust_zenoh_802154_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-802154 && cp target/release/libzenoh_802154.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_802154.a', meson.current_build_dir() / 'rust_zenoh_802154_trigger.c'
  ]
)

# ── zenoh-ui ─────────────────────────────────────────────────────────────
# Rust implementation of the Zenoh UI backend.
rust_zenoh_ui_lib = custom_target(
  'rust_zenoh_ui_lib',
  input: 'rust/zenoh-ui/src/lib.rs',
  output: ['libzenoh_ui.a', 'rust_zenoh_ui_trigger.c'],
  command: [
    'sh', '-c',
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-ui && cp target/release/libzenoh_ui.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libzenoh_ui.a', meson.current_build_dir() / 'rust_zenoh_ui_trigger.c'
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

# ── test-qom-device ────────────────────────────────────────────────────────
# Test device for Phase 19 pure Rust QOM registration
rust_test_qom_device_lib = custom_target(
  'rust_test_qom_device_lib',
  input: 'rust/test-qom-device/src/lib.rs',
  output: ['libtest_qom_device.a', 'rust_test_qom_device_trigger.c'],
  command: [
    'sh', '-c',
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && cargo build --release --target-dir "$RUST_DIR/target" -p test-qom-device && cp target/release/libtest_qom_device.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("test-rust-device");
EOF
''',
    'sh', meson.project_source_root() / 'hw/virtmcu/rust', meson.current_build_dir() / 'libtest_qom_device.a', meson.current_build_dir() / 'rust_test_qom_device_trigger.c'
  ]
)
test_qom_device_ss = ss.source_set()
test_qom_device_ss.add(virtmcu_ffi_dep)
test_qom_device_ss.add(virtmcu_rust_ffi_file)
test_qom_device_ss.add(rust_test_qom_device_lib[1])
test_qom_device_ss.add(rust_test_qom_device_lib[0])
test_qom_device_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libtest_qom_device.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'test-qom-device': test_qom_device_ss}

# ── Add new devices above this line ──────────────────────────────────────────
modules += {'hw-virtmcu': hw_virtmcu_modules}

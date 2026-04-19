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
    'RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p rust_dummy && cp target/release/librust_dummy.a "$OUT_A" && echo "/* generated */" > "$OUT_TRIG"',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'librust_dummy.a', meson.current_build_dir() / 'rust_dummy_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
rust_dummy_ss = ss.source_set()
rust_dummy_ss.add(files('rust/rust-dummy/rust-dummy.c'))
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
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-clock && cp target/release/libzenoh_clock.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("zenoh-clock");
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libzenoh_clock.a', meson.current_build_dir() / 'rust_zenoh_clock_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
zenoh_clock_ss = ss.source_set()
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
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-netdev && cp target/release/libzenoh_netdev.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("zenoh-netdev");
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libzenoh_netdev.a', meson.current_build_dir() / 'rust_zenoh_netdev_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
zenoh_netdev_ss = ss.source_set()
zenoh_netdev_ss.add(rust_zenoh_netdev_lib[1])
zenoh_netdev_ss.add(rust_zenoh_netdev_lib[0])
zenoh_netdev_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libzenoh_netdev.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'zenoh-netdev': zenoh_netdev_ss}

# ── zenoh-chardev ──────────────────────────────────────────────────────────
# Rust implementation of the Zenoh character device backend.
rust_zenoh_chardev_lib = custom_target(
  'rust_zenoh_chardev_lib',
  input: 'rust/zenoh-chardev/src/lib.rs',
  output: ['libzenoh_chardev.a', 'rust_zenoh_chardev_trigger.c'],
  command: [
    'sh', '-c',
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-chardev && cp target/release/libzenoh_chardev.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
#include "qapi/qapi-types-char.h"
module_obj("chardev-zenoh");
int get_chardev_backend_kind_zenoh(void);
int get_chardev_backend_kind_zenoh(void) {
    return CHARDEV_BACKEND_KIND_ZENOH;
}
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libzenoh_chardev.a', meson.current_build_dir() / 'rust_zenoh_chardev_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
zenoh_chardev_ss = ss.source_set()
zenoh_chardev_ss.add(rust_zenoh_chardev_lib[1])
zenoh_chardev_ss.add(rust_zenoh_chardev_lib[0])
zenoh_chardev_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libzenoh_chardev.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'zenoh-chardev': zenoh_chardev_ss}


# ── zenoh-telemetry ────────────────────────────────────────────────────────
# Rust implementation of the Zenoh telemetry backend.
rust_zenoh_telemetry_lib = custom_target(
  'rust_zenoh_telemetry_lib',
  input: 'rust/zenoh-telemetry/src/lib.rs',
  output: ['libzenoh_telemetry.a', 'rust_zenoh_telemetry_trigger.c'],
  command: [
    'sh', '-c',
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-telemetry && cp target/release/libzenoh_telemetry.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("zenoh-telemetry");
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libzenoh_telemetry.a', meson.current_build_dir() / 'rust_zenoh_telemetry_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
zenoh_telemetry_ss = ss.source_set()
zenoh_telemetry_ss.add(rust_zenoh_telemetry_lib[1])
zenoh_telemetry_ss.add(rust_zenoh_telemetry_lib[0])
zenoh_telemetry_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libzenoh_telemetry.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'zenoh-telemetry': zenoh_telemetry_ss}


# ── zenoh-actuator ────────────────────────────────────────────────────────
# Rust implementation of the Zenoh actuator backend.
rust_zenoh_actuator_lib = custom_target(
  'rust_zenoh_actuator_lib',
  input: 'rust/zenoh-actuator/src/lib.rs',
  output: ['libzenoh_actuator.a', 'rust_zenoh_actuator_trigger.c'],
  command: [
    'sh', '-c',
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-actuator && cp target/release/libzenoh_actuator.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("zenoh-actuator");
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libzenoh_actuator.a', meson.current_build_dir() / 'rust_zenoh_actuator_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
zenoh_actuator_ss = ss.source_set()
zenoh_actuator_ss.add(rust_zenoh_actuator_lib[1])
zenoh_actuator_ss.add(rust_zenoh_actuator_lib[0])
zenoh_actuator_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libzenoh_actuator.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'zenoh-actuator': zenoh_actuator_ss}


# ── zenoh-802154 ─────────────────────────────────────────────────────────
# Rust implementation of the Zenoh 802.15.4 radio.
rust_zenoh_802154_lib = custom_target(
  'rust_zenoh_802154_lib',
  input: 'rust/zenoh-802154/src/lib.rs',
  output: ['libzenoh_802154.a', 'rust_zenoh_802154_trigger.c'],
  command: [
    'sh', '-c',
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-802154 && cp target/release/libzenoh_802154.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("zenoh-802154");
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libzenoh_802154.a', meson.current_build_dir() / 'rust_zenoh_802154_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
zenoh_802154_ss = ss.source_set()
zenoh_802154_ss.add(rust_zenoh_802154_lib[1])
zenoh_802154_ss.add(rust_zenoh_802154_lib[0])
zenoh_802154_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libzenoh_802154.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'zenoh-802154': zenoh_802154_ss}


# ── zenoh-ui ─────────────────────────────────────────────────────────────
# Rust implementation of the Zenoh UI backend.
rust_zenoh_ui_lib = custom_target(
  'rust_zenoh_ui_lib',
  input: 'rust/zenoh-ui/src/lib.rs',
  output: ['libzenoh_ui.a', 'rust_zenoh_ui_trigger.c'],
  command: [
    'sh', '-c',
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p zenoh-ui && cp target/release/libzenoh_ui.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("zenoh-ui");
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libzenoh_ui.a', meson.current_build_dir() / 'rust_zenoh_ui_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
zenoh_ui_ss = ss.source_set()
zenoh_ui_ss.add(rust_zenoh_ui_lib[1])
zenoh_ui_ss.add(rust_zenoh_ui_lib[0])
zenoh_ui_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libzenoh_ui.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'zenoh-ui': zenoh_ui_ss}


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
    '''RUST_DIR="$1"; OUT_A="$2"; OUT_TRIG="$3"; cd "$RUST_DIR" && QEMU_SRC_DIR="$4" QEMU_BUILD_DIR="$5" cargo build --release --target-dir "$RUST_DIR/target" -p test-qom-device && cp target/release/libtest_qom_device.a "$OUT_A"
cat <<EOF > "$OUT_TRIG"
#include "qemu/osdep.h"
#include "qemu/module.h"
module_obj("test-rust-device");
EOF
''',
    'sh', meson.current_source_dir() / 'rust', meson.current_build_dir() / 'libtest_qom_device.a', meson.current_build_dir() / 'rust_test_qom_device_trigger.c', meson.project_source_root(), meson.project_build_root()
  ]
)
test_qom_device_ss = ss.source_set()
test_qom_device_ss.add(rust_test_qom_device_lib[1])
test_qom_device_ss.add(rust_test_qom_device_lib[0])
test_qom_device_ss.add(declare_dependency(
  link_args: ['-Wl,--whole-archive', meson.current_build_dir() / 'libtest_qom_device.a', '-Wl,--no-whole-archive']
))
hw_virtmcu_modules += {'test-qom-device': test_qom_device_ss}

# ── Add new devices above this line ──────────────────────────────────────────
modules += {'hw-virtmcu': hw_virtmcu_modules}

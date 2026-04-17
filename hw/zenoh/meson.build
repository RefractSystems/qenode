# Define individual source sets for each Zenoh device to support dynamic loading via -device.

# ── zenoh-netdev ─────────────────────────────────────────────────────────────
zenoh_netdev_ss = ss.source_set()
zenoh_netdev_ss.add(files('zenoh-netdev.c'))
zenoh_netdev_ss.add(virtmcu_rust_ffi_file, virtmcu_ffi_dep)
zenoh_netdev_ss.add(rust_zenoh_netdev_lib[1])
zenoh_netdev_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / '../libzenoh_netdev.a']
))
hw_virtmcu_modules += {'zenoh-netdev': zenoh_netdev_ss}

# ── zenoh-chardev ────────────────────────────────────────────────────────────
zenoh_chardev_ss = ss.source_set()
zenoh_chardev_ss.add(files('zenoh-chardev.c'))
zenoh_chardev_ss.add(virtmcu_rust_ffi_file, virtmcu_ffi_dep)
zenoh_chardev_ss.add(rust_zenoh_chardev_lib[1])
zenoh_chardev_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / '../libzenoh_chardev.a']
))
hw_virtmcu_modules += {'zenoh-chardev': zenoh_chardev_ss}

# ── zenoh-telemetry ──────────────────────────────────────────────────────────
zenoh_telemetry_ss = ss.source_set()
zenoh_telemetry_ss.add(files('zenoh-telemetry.c'))
zenoh_telemetry_ss.add(virtmcu_rust_ffi_file, virtmcu_ffi_dep)
zenoh_telemetry_ss.add(rust_zenoh_telemetry_lib[1])
zenoh_telemetry_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / '../libzenoh_telemetry.a']
))
hw_virtmcu_modules += {'zenoh-telemetry': zenoh_telemetry_ss}

# ── zenoh-actuator ───────────────────────────────────────────────────────────
zenoh_actuator_ss = ss.source_set()
zenoh_actuator_ss.add(files('zenoh-actuator.c'))
zenoh_actuator_ss.add(virtmcu_rust_ffi_file, virtmcu_ffi_dep)
zenoh_actuator_ss.add(rust_zenoh_actuator_lib[1])
zenoh_actuator_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / '../libzenoh_actuator.a']
))
hw_virtmcu_modules += {'zenoh-actuator': zenoh_actuator_ss}

# ── zenoh-802154 ────────────────────────────────────────────────────────────
zenoh_802154_ss = ss.source_set()
zenoh_802154_ss.add(files('zenoh-802154.c'))
zenoh_802154_ss.add(virtmcu_rust_ffi_file, virtmcu_ffi_dep)
zenoh_802154_ss.add(rust_zenoh_802154_lib[1])
zenoh_802154_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / '../libzenoh_802154.a']
))
hw_virtmcu_modules += {'zenoh-802154': zenoh_802154_ss}

# ── zenoh-ui ─────────────────────────────────────────────────────────────────
zenoh_ui_ss = ss.source_set()
zenoh_ui_ss.add(files('zenoh-ui.c'))
zenoh_ui_ss.add(virtmcu_rust_ffi_file, virtmcu_ffi_dep)
zenoh_ui_ss.add(rust_zenoh_ui_lib[1])
zenoh_ui_ss.add(declare_dependency(
  link_args: [meson.current_build_dir() / '../libzenoh_ui.a']
))
hw_virtmcu_modules += {'zenoh-ui': zenoh_ui_ss}

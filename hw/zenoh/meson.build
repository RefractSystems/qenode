zenoh_monolith_ss = ss.source_set()
zenoh_monolith_ss.add(files('zenoh-netdev.c', 'zenoh-chardev.c', 'zenoh-actuator.c', 'zenoh-telemetry.c', 'zenoh-802154.c', 'zenoh-ui.c'))
zenoh_monolith_ss.add(virtmcu_rust_ffi_file)
zenoh_monolith_ss.add(rust_zenoh_netdev_lib[1], rust_zenoh_chardev_lib[1], rust_zenoh_telemetry_lib[1], rust_zenoh_actuator_lib[1], rust_zenoh_802154_lib[1], rust_zenoh_ui_lib[1])
zenoh_monolith_ss.add(declare_dependency(
  link_args: [
    meson.current_build_dir() / '../libzenoh_netdev.a',
    meson.current_build_dir() / '../libzenoh_chardev.a',
    meson.current_build_dir() / '../libzenoh_telemetry.a',
    meson.current_build_dir() / '../libzenoh_actuator.a',
    meson.current_build_dir() / '../libzenoh_802154.a',
    meson.current_build_dir() / '../libzenoh_ui.a'
  ]
))
hw_virtmcu_modules += {'zenoh': zenoh_monolith_ss}

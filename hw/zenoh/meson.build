zenoh_dir = meson.project_source_root() / '../../third_party/zenoh-c'
zenoh_lib = cc.find_library('zenohc', dirs: zenoh_dir)
zenoh_dep = declare_dependency(
  dependencies: zenoh_lib,
  compile_args: ['-I' + zenoh_dir / 'include']
)

zenoh_clock_ss = ss.source_set()
zenoh_clock_ss.add(files('zenoh-clock.c', 'zenoh-netdev.c'))
zenoh_clock_ss.add(zenoh_dep)
hw_virtmcu_modules += {'zenoh': zenoh_clock_ss}

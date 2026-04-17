zenoh_dir = meson.project_source_root() / '../zenoh-c'
zenoh_lib = cc.find_library('zenohc', dirs: zenoh_dir / 'lib')
zenoh_dep = declare_dependency(
  dependencies: zenoh_lib,
  compile_args: ['-I' + zenoh_dir / 'include']
)

# flatcc search: /usr/local (Docker/System) or ../flatcc (Local/setup-qemu.sh)
flatcc_inc = []
flatcc_lib_dirs = []
flatcc_bin = find_program('flatcc', required: false)

local_flatcc_dir = meson.project_source_root() / '../flatcc'
if cc.has_header('flatcc/flatcc_builder.h')
  # System installed
  flatcc_inc = []
  flatcc_lib_dirs = []
elif cc.has_header('flatcc/flatcc_builder.h', args: '-I' + local_flatcc_dir / 'include')
  # Local build
  flatcc_inc = [local_flatcc_dir / 'include']
  flatcc_lib_dirs = [local_flatcc_dir / 'lib']
  if not flatcc_bin.found()
    flatcc_bin = find_program(local_flatcc_dir / 'bin/flatcc')
  endif
endif

flatcc_cargs = []
if flatcc_inc.length() > 0
  flatcc_cargs += ['-I' + flatcc_inc[0]]
endif

flatcc_lib = cc.find_library('flatccrt', dirs: flatcc_lib_dirs)
flatcc_dep = declare_dependency(
  dependencies: flatcc_lib,
  compile_args: flatcc_cargs
)

telemetry_fbs = files('telemetry.fbs')
telemetry_h = custom_target('telemetry_fbs',
  input: telemetry_fbs,
  output: ['telemetry_builder.h', 'telemetry_reader.h'],
  command: [flatcc_bin, '-a', '-o', '@OUTDIR@', '@INPUT@']
)

zenoh_monolith_ss = ss.source_set()
zenoh_monolith_ss.add(files('zenoh-netdev.c', 'zenoh-chardev.c', 'zenoh-actuator.c', 'zenoh-telemetry.c', 'zenoh-802154.c', 'zenoh-ui.c'))
zenoh_monolith_ss.add(virtmcu_rust_ffi_file)
zenoh_monolith_ss.add(zenoh_dep, flatcc_dep)
zenoh_monolith_ss.add(telemetry_h)
zenoh_monolith_ss.add(rust_zenoh_netdev_lib[1], rust_zenoh_chardev_lib[1])
zenoh_monolith_ss.add(declare_dependency(
  link_args: [
    meson.current_build_dir() / '../libzenoh_netdev.a',
    meson.current_build_dir() / '../libzenoh_chardev.a'
  ]
))
hw_virtmcu_modules += {'zenoh': zenoh_monolith_ss}

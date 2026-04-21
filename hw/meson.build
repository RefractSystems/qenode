# virtmcu hardware peripheral modules.
#
# This file lives at hw/meson.build inside the virtmcu repo.
# scripts/setup-qemu.sh creates a symlink:
#   <qemu-src>/hw/virtmcu  →  <virtmcu-repo>/hw
# and appends `subdir('virtmcu')` to <qemu-src>/hw/meson.build.
#
# With --enable-modules each entry in hw_virtmcu_modules compiles to a
# standalone shared library (hw-virtmcu-<name>.so) placed in the
# QEMU module dir.

hw_virtmcu_modules = {}

# Common FFI helpers for Rust-backed modules
hw_virtmcu_inc = include_directories('.', 'misc')

rust_modules = [
    {'pkg': 'rust-dummy', 'lib': 'librust_dummy.a', 'obj': 'rust-dummy'},
    {'pkg': 'zenoh-clock', 'lib': 'libzenoh_clock.a', 'obj': 'zenoh-clock'},
    {'pkg': 'zenoh-netdev', 'lib': 'libzenoh_netdev.a', 'obj': 'zenoh-netdev'},
    {'pkg': 'zenoh-canfd', 'lib': 'libzenoh_canfd.a', 'obj': 'can-host-zenoh'},
    {
        'pkg': 'zenoh-chardev',
        'lib': 'libzenoh_chardev.a',
        'obj': 'chardev-zenoh',
        'extra_c': 'int get_chardev_backend_kind_zenoh(void); int get_chardev_backend_kind_zenoh(void) { return CHARDEV_BACKEND_KIND_ZENOH; }',
        'extra_inc': '#include "qapi/qapi-types-char.h"',
    },
    {
        'pkg': 'zenoh-telemetry',
        'lib': 'libzenoh_telemetry.a',
        'obj': 'zenoh-telemetry',
    },
    {
        'pkg': 'zenoh-actuator',
        'lib': 'libzenoh_actuator.a',
        'obj': 'zenoh-actuator',
    },
    {'pkg': 'zenoh-802154', 'lib': 'libzenoh_802154.a', 'obj': 'zenoh-802154'},
    {'pkg': 'zenoh-ui', 'lib': 'libzenoh_ui.a', 'obj': 'zenoh-ui'},
    {
        'pkg': 'test-qom-device',
        'lib': 'libtest_qom_device.a',
        'objs': ['test-rust-device', 'spi-echo', 'uart-echo'],
    },
    {
        'pkg': 'mmio-socket-bridge',
        'lib': 'libmmio_socket_bridge.a',
        'obj': 'mmio-socket-bridge',
    },
    {
        'pkg': 'remote-port',
        'lib': 'libremote_port.a',
        'obj': 'remote-port-bridge',
    },
    {
        'pkg': 'zenoh-flexray',
        'lib': 'libzenoh_flexray.a',
        'obj': 'zenoh-flexray',
    },
    {
        'pkg': 's32k144-lpuart',
        'lib': 'libs32k144_lpuart.a',
        'obj': 's32k144-lpuart',
    },
    {'pkg': 'virtmcu-wifi', 'lib': 'libvirtmcu_wifi.a', 'obj': 'virtmcu-wifi'},
    {'pkg': 'zenoh-spi', 'lib': 'libzenoh_spi.a', 'obj': 'zenoh-spi'},
]

rust_libs_outputs = []
rust_build_args = []
foreach m : rust_modules
    rust_libs_outputs += m['lib']
    rust_build_args += m['pkg'] + ':' + m['lib']
endforeach

# Get all Rust sources to ensure correct dependency tracking
rust_sources_res = run_command(
    'find',
    meson.current_source_dir() / 'rust',
    '-path',
    meson.current_source_dir() / 'rust/target',
    '-prune',
    '-o',
    '-name',
    '*.rs',
    '-print',
    check: true,
)
rust_sources = rust_sources_res.stdout().strip().split('\n')
message('Found ' + rust_sources.length().to_string() + ' Rust source files')
if rust_sources.length() < 10
    warning(
        'Very few Rust source files found! find output: ' + rust_sources_res.stdout(),
    )
endif

# Single workspace build to avoid Cargo.lock contention and enable native parallelism
message(
    'Configuring Rust virtmcu modules with QEMU_SRC_DIR=' + meson.project_source_root(),
)
rust_libs = custom_target(
    'rust_virtmcu_libs',
    input: rust_sources + [
        meson.current_source_dir() / '../Cargo.lock',
        meson.current_source_dir() / '../Cargo.toml',
    ],
    output: rust_libs_outputs,
    build_by_default: true,
    env: {
        'QEMU_SRC_DIR': meson.project_source_root(),
        'QEMU_BUILD_DIR': meson.project_build_root(),
    },
    command: [
        meson.current_source_dir() / 'rust/build.sh',
        meson.current_source_dir() / 'rust',
        meson.current_build_dir() / 'target',
        meson.current_build_dir(),
        rust_build_args,
    ],
)

i = 0
foreach m : rust_modules
    pkg_lib = rust_libs[i]
    i += 1

    message('Registering Rust module: ' + m['pkg'])
    # Generate trigger .c file separately per module
    trigger_c = custom_target(
        m['pkg'] + '_trigger_c',
        output: m['pkg'] + '_trigger.c',
        depends: rust_libs,
        command: [
            meson.current_source_dir() / 'gen_trigger.sh',
            '@OUTPUT@',
            m.get('obj', ''),
            m.get('extra_inc', ''),
            m.get('extra_c', ''),
            m.get('objs', []),
        ],
    )

    ss_name = ss.source_set()
    ss_name.add(trigger_c)
    ss_name.add(
        declare_dependency(
            sources: pkg_lib,
            link_args: [
                '-Wl,--whole-archive',
                pkg_lib.full_path(),
                '-Wl,--no-whole-archive',
            ],
        ),
    )
    hw_virtmcu_modules += {m['pkg']: ss_name}
endforeach


# Educational C dummy module (must be added before we update modules dict)
ss_educational_dummy = ss.source_set()
ss_educational_dummy.add(files('misc/educational-dummy.c'))
hw_virtmcu_modules += {'educational-dummy': ss_educational_dummy}

message('Adding hw-virtmcu group to modules dictionary')
modules += {'hw-virtmcu': hw_virtmcu_modules}

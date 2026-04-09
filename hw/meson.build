# qenode hardware peripheral modules.
#
# This file lives at hw/meson.build inside the qenode repo.
# scripts/setup-qemu.sh creates a symlink:
#   <qemu-src>/hw/qenode  →  <qenode-repo>/hw
# and appends `subdir('qenode')` to <qemu-src>/hw/meson.build.
#
# With --enable-modules each entry in hw_qenode_modules compiles to a
# standalone shared library (hw-qenode-<name>.so / .dylib) placed in the
# QEMU module dir.  Without modules it is linked statically into qemu-system-*.
#
# Adding a new device:
#   1. Create hw/<name>/<name>.c following the pattern in hw/dummy/dummy.c.
#   2. Add a source_set and register it below.

hw_qenode_modules = {}

# ── dummy ─────────────────────────────────────────────────────────────────────
# Minimal QOM SysBusDevice.  Always included — it is the smoke-test for the
# dynamic loading pipeline and the template for all new devices.
dummy_ss = ss.source_set()
dummy_ss.add(files('dummy/dummy.c'))
hw_qenode_modules += {'dummy': dummy_ss}

# ── Add new devices above this line ──────────────────────────────────────────
modules += {'hw-qenode': hw_qenode_modules}

#!/usr/bin/env python3
# ==============================================================================
# apply_zenoh_qapi.py
#
# Inject Zenoh netdev and chardev entries into QEMU's QAPI schema files.
#
# QEMU's QAPI schemas (qapi/net.json, qapi/char.json) define the set of
# recognised -netdev and -chardev backends.  Our native Zenoh QOM plugins
# (hw/virtmcu/zenoh/zenoh-netdev.c and zenoh-chardev.c) reference the
# QAPI-generated types NET_CLIENT_DRIVER_ZENOH, NetdevZenohOptions,
# CHARDEV_BACKEND_KIND_ZENOH, and ChardevZenohOptions.  Without these
# additions the generated C headers are missing those symbols and the build
# fails with "undeclared identifier" errors.
#
# This script is idempotent: each insertion is guarded by a check for the
# zenoh token before modifying the file.
# ==============================================================================

import os
import sys
from pathlib import Path


def patch_file(path, marker, insertion, guard, after=True):
    """Insert *insertion* immediately before/after the first occurrence of
    *marker* in *path*.  Returns True if the file was changed.

    *guard* is a unique substring that, if already present in the file,
    indicates the patch was already applied (idempotency check).
    """
    with Path(path).open() as f:
        content = f.read()
    if guard in content:
        return False  # already applied
    if marker not in content:
        print(f"  WARNING: marker not found in {os.path.relpath(path)}: {marker!r}")
        return False
    if after:
        content = content.replace(marker, marker + insertion, 1)
    else:
        content = content.replace(marker, insertion + marker, 1)
    with Path(path).open("w") as f:
        f.write(content)
    print(f"  patched {os.path.relpath(path)}")
    return True


def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = Path(sys.argv[1]).resolve()
    net_json = Path(qemu) / "qapi" / "net.json"
    char_json = Path(qemu) / "qapi" / "char.json"
    qom_json = Path(qemu) / "qapi" / "qom.json"

    for p in (net_json, char_json, qom_json):
        if not Path(p).exists():
            print(f"ERROR: {p} not found")
            sys.exit(1)

    # ── qapi/net.json ─────────────────────────────────────────────────────────

    # 1. Add 'zenoh' to NetClientDriver enum documentation
    patch_file(
        net_json,
        marker="# @vhost-vdpa: since 5.1",
        insertion="\n#\n# @zenoh: since 11.0",
        guard="# @zenoh: since",
        after=True,
    )

    # 2. Add 'zenoh' to NetClientDriver enum (after 'vhost-vdpa')
    patch_file(
        net_json,
        marker="'vhost-vdpa',",
        insertion="\n            'zenoh',",
        guard="\n            'zenoh',",
        after=True,
    )

    # 3. Add NetdevZenohOptions struct (before the vmnet-host docstring block)
    netdev_struct = """
##
# @NetdevZenohOptions:
#
# virtmcu: Zenoh virtual clock network backend
#
# @node: The zenoh node ID
# @router: The zenoh router address (optional)
# @topic: The zenoh topic to publish/subscribe to (optional)
#
# Since: 11.0
##
{ 'struct': 'NetdevZenohOptions',
  'data': {
    'node': 'str',
    '*router': 'str',
    '*topic': 'str' } }

"""
    patch_file(
        net_json,
        marker="##\n# @NetdevVmnetHostOptions:",
        insertion=netdev_struct,
        guard="{ 'struct': 'NetdevZenohOptions',",
        after=False,
    )

    # 4. Add 'zenoh' discriminator to Netdev union (before 'vmnet-host')
    patch_file(
        net_json,
        marker="    'vhost-vdpa': 'NetdevVhostVDPAOptions',",
        insertion="\n    'zenoh':    'NetdevZenohOptions',",
        guard="'zenoh':    'NetdevZenohOptions',",
        after=True,
    )

    # ── qapi/char.json ────────────────────────────────────────────────────────

    # 5. Add 'zenoh' to ChardevBackendKind enum documentation
    patch_file(
        char_json,
        marker="# @ringbuf: memory ring buffer (since 1.6)",
        insertion="\n#\n# @zenoh: zenoh virtual clock backend (since 11.0)",
        guard="# @zenoh: zenoh virtual clock backend (since",
        after=True,
    )

    # 6. Add 'zenoh' to ChardevBackendKind enum (after 'ringbuf', before 'memory')
    patch_file(
        char_json,
        marker="            'ringbuf',",
        insertion="\n            'zenoh',",
        guard="\n            'zenoh',",
        after=True,
    )

    # 7. Add ChardevZenohOptions + ChardevZenohWrapper structs
    #    (before the existing ChardevFileWrapper docstring block)
    chardev_structs = """
##
# @ChardevZenohOptions:
#
# virtmcu: Zenoh virtual clock chardev backend
#
# @node: The zenoh node ID
# @router: The zenoh router address (optional)
# @topic: The zenoh topic to publish/subscribe to (optional)
#
# Since: 11.0
##
{ 'struct': 'ChardevZenohOptions',
  'base': 'ChardevCommon',
  'data': {
    'node': 'str',
    '*router': 'str',
    '*topic': 'str' } }

##
# @ChardevZenohWrapper:
#
# @data: Configuration info for zenoh chardevs
#
# Since: 11.0
##
{ 'struct': 'ChardevZenohWrapper',
  'data': { 'data': 'ChardevZenohOptions' } }


"""
    patch_file(
        char_json,
        marker="##\n# @ChardevFileWrapper:",
        insertion=chardev_structs,
        guard="{ 'struct': 'ChardevZenohOptions',",
        after=False,
    )

    # 8. Add 'zenoh' discriminator to ChardevBackend union (before 'memory')
    patch_file(
        char_json,
        marker="'ringbuf': 'ChardevRingbufWrapper',",
        insertion="\n            'zenoh': 'ChardevZenohWrapper',",
        guard="'zenoh': 'ChardevZenohWrapper'",
        after=False,
    )

    print("apply_zenoh_qapi.py: done")

    # ── qapi/qom.json ─────────────────────────────────────────────────────────

    can_host_zenoh_struct = """
##
# @CanHostZenohProperties:
#
# Properties for can-host-zenoh objects.
#
# @node: The zenoh node ID
# @router: The zenoh router address (optional)
# @topic: The zenoh topic to publish/subscribe to
#
# @canbus: object ID of the can-bus object to connect to the host
#     interface
#
# Since: 11.0
##
{ 'struct': 'CanHostZenohProperties',
  'data': { 'canbus': 'str',
            'node': 'str',
            '*router': 'str',
            'topic': 'str' } }

"""
    # 9. Add CanHostZenohProperties struct (before ColoCompareProperties)
    patch_file(
        qom_json,
        marker="##\n# @ColoCompareProperties:",
        insertion=can_host_zenoh_struct,
        guard="CanHostZenohProperties",
        after=False,
    )

    # 10. Add 'can-host-zenoh' to ObjectType enum (before colo-compare)
    patch_file(
        qom_json,
        marker="    'colo-compare',",
        insertion="    'can-host-zenoh',\n",
        guard="'can-host-zenoh',",
        after=False,
    )

    # 11. Add 'can-host-zenoh' discriminator to ObjectOptions (before colo-compare)
    patch_file(
        qom_json,
        marker="      'colo-compare':               'ColoCompareProperties',",
        insertion="      'can-host-zenoh':             'CanHostZenohProperties',\n",
        guard="'can-host-zenoh':             'CanHostZenohProperties',",
        after=False,
    )


if __name__ == "__main__":
    main()

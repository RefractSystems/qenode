#!/usr/bin/env python3
# ==============================================================================
# apply_zenoh_qapi.py
#
# Inject Zenoh netdev and chardev entries into QEMU's QAPI schema files.
#
# QEMU's QAPI schemas (qapi/net.json, qapi/char.json) define the set of
# recognised -netdev and -chardev backends.  Our native Zenoh QOM plugins
# (hw/virtmcu/zenoh/netdev.c and chardev.c) reference the
# QAPI-generated types NET_CLIENT_DRIVER_ZENOH, NetdevZenohOptions,
# CHARDEV_BACKEND_KIND_ZENOH, and ChardevZenohOptions.  Without these
# additions the generated C headers are missing those symbols and the build
# fails with "undeclared identifier" errors.
#
# This script is idempotent: each insertion is guarded by a check for the
# zenoh token before modifying the file.
# ==============================================================================

import logging
import os
import sys
from pathlib import Path

logger = logging.getLogger(__name__)


def patch_file(path: str | Path, marker: str, insertion: str, guard: str, after: bool = True) -> bool:
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
        logger.info(f"  WARNING: marker not found in {os.path.relpath(path)}: {marker!r}")
        return False
    if after:
        content = content.replace(marker, marker + insertion, 1)
    else:
        content = content.replace(marker, insertion + marker, 1)
    with Path(path).open("w") as f:
        f.write(content)
    logger.info(f"  patched {os.path.relpath(path)}")
    return True


def main() -> None:
    if len(sys.argv) != 2:
        logger.info(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = Path(sys.argv[1]).resolve()
    net_json = Path(qemu) / "qapi" / "net.json"
    char_json = Path(qemu) / "qapi" / "char.json"
    qom_json = Path(qemu) / "qapi" / "qom.json"

    for p in (net_json, char_json, qom_json):
        if not Path(p).exists():
            logger.error(f"ERROR: {p} not found")
            sys.exit(1)

    # ── qapi/net.json ─────────────────────────────────────────────────────────

    # 1. Add 'virtmcu' to NetClientDriver enum documentation
    patch_file(
        net_json,
        marker="# @vhost-vdpa: since 5.1",
        insertion="\n#\n# @virtmcu: since 11.0",
        guard="# @virtmcu: since",
        after=True,
    )

    # 2. Add 'virtmcu' to NetClientDriver enum (after 'vhost-vdpa')
    patch_file(
        net_json,
        marker="'vhost-vdpa',",
        insertion="\n            'virtmcu',",
        guard="\n            'virtmcu',",
        after=True,
    )

    # 3. Add NetdevVirtmcuOptions struct (before the vmnet-host docstring block)
    netdev_struct = """
##
# @NetdevVirtmcuOptions:
#
# virtmcu: Virtual clock network backend
#
# @node: The node ID
# @transport: The transport to use (zenoh or unix) (optional)
# @router: The zenoh router address (optional)
# @topic: The topic to publish/subscribe to (optional)
# @max-backlog: Maximum number of frames in the RX backlog
#     (default: 256) (optional)
#
# Since: 11.0
##
{ 'struct': 'NetdevVirtmcuOptions',
  'data': {
    'node': 'str',
    '*transport': 'str',
    '*router': 'str',
    '*topic': 'str',
    '*max-backlog': 'size' } }

"""
    patch_file(
        net_json,
        marker="##\n# @NetdevVmnetHostOptions:",
        insertion=netdev_struct,
        guard="{ 'struct': 'NetdevVirtmcuOptions',",
        after=False,
    )

    # 4. Add 'virtmcu' discriminator to Netdev union (before 'vmnet-host')
    patch_file(
        net_json,
        marker="    'vhost-vdpa': 'NetdevVhostVDPAOptions',",
        insertion="\n    'virtmcu':    'NetdevVirtmcuOptions',",
        guard="'virtmcu':    'NetdevVirtmcuOptions',",
        after=True,
    )

    # ── qapi/char.json ────────────────────────────────────────────────────────

    # 5. Add 'virtmcu' to ChardevBackendKind enum documentation
    patch_file(
        char_json,
        marker="# @ringbuf: memory ring buffer (since 1.6)",
        insertion="\n#\n# @virtmcu: virtmcu virtual clock backend (since 11.0)",
        guard="# @virtmcu: virtmcu virtual clock backend (since",
        after=True,
    )

    # 6. Add 'virtmcu' to ChardevBackendKind enum (after 'ringbuf', before 'memory')
    patch_file(
        char_json,
        marker="            'ringbuf',",
        insertion="\n            'virtmcu',",
        guard="\n            'virtmcu',",
        after=True,
    )

    # 7. Add ChardevVirtmcuOptions + ChardevVirtmcuWrapper structs
    #    (before the existing ChardevFileWrapper docstring block)
    chardev_structs = """
##
# @ChardevVirtmcuOptions:
#
# virtmcu: Virtual clock chardev backend
#
# @node: The node ID
# @transport: The transport to use (zenoh or unix) (optional)
# @router: The zenoh router address (optional)
# @topic: The topic to publish/subscribe to (optional)
# @max-backlog: Maximum number of bytes in the RX backlog
#     (default: 256) (optional)
#
# Since: 11.0
##
{ 'struct': 'ChardevVirtmcuOptions',
  'base': 'ChardevCommon',
  'data': {
    'node': 'str',
    '*transport': 'str',
    '*router': 'str',
    '*topic': 'str',
    '*max-backlog': 'size' } }

##
# @ChardevVirtmcuWrapper:
#
# @data: Configuration info for virtmcu chardevs
#
# Since: 11.0
##
{ 'struct': 'ChardevVirtmcuWrapper',
  'data': { 'data': 'ChardevVirtmcuOptions' } }


"""
    patch_file(
        char_json,
        marker="##\n# @ChardevFileWrapper:",
        insertion=chardev_structs,
        guard="{ 'struct': 'ChardevVirtmcuOptions',",
        after=False,
    )

    # 8. Add 'virtmcu' discriminator to ChardevBackend union (before 'memory')
    patch_file(
        char_json,
        marker="'ringbuf': 'ChardevRingbufWrapper',",
        insertion="\n            'virtmcu': 'ChardevVirtmcuWrapper',",
        guard="'virtmcu': 'ChardevVirtmcuWrapper'",
        after=False,
    )

    logger.info("apply_zenoh_qapi.py: done")

    # ── qapi/qom.json ─────────────────────────────────────────────────────────

    can_host_virtmcu_struct = """
##
# @CanHostVirtmcuProperties:
#
# Properties for can-host-virtmcu objects.
#
# @node: The node ID
# @transport: The transport to use (zenoh or unix) (optional)
# @router: The zenoh router address (optional)
# @topic: The topic to publish/subscribe to
#
# @canbus: object ID of the can-bus object to connect to the host
#     interface
#
# Since: 11.0
##
{ 'struct': 'CanHostVirtmcuProperties',
  'data': { 'canbus': 'str',
            'node': 'str',
            '*transport': 'str',
            '*router': 'str',
            'topic': 'str' } }

"""
    # 9. Add CanHostVirtmcuProperties struct (before ColoCompareProperties)
    patch_file(
        qom_json,
        marker="##\n# @ColoCompareProperties:",
        insertion=can_host_virtmcu_struct,
        guard="CanHostVirtmcuProperties",
        after=False,
    )

    # 10. Add 'can-host-virtmcu' to ObjectType enum (before colo-compare)
    patch_file(
        qom_json,
        marker="    'colo-compare',",
        insertion="    'can-host-virtmcu',\n",
        guard="'can-host-virtmcu',",
        after=False,
    )

    # 11. Add 'can-host-virtmcu' discriminator to ObjectOptions (before colo-compare)
    patch_file(
        qom_json,
        marker="      'colo-compare':               'ColoCompareProperties',",
        insertion="      'can-host-virtmcu':             'CanHostVirtmcuProperties',\n",
        guard="'can-host-virtmcu':             'CanHostVirtmcuProperties',",
        after=False,
    )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()

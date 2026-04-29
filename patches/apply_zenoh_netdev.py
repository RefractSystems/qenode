#!/usr/bin/env python3
import logging
import sys
from pathlib import Path

logger = logging.getLogger(__name__)


def patch_file(filepath, marker, insertion, after=False):
    with Path(filepath).open() as f:
        content = f.read()
    if insertion in content:
        return False
    idx = content.find(marker)
    if idx == -1:
        logger.error(f"Error: Could not find marker '{marker}' in {filepath}")
        sys.exit(1)
    if after:
        idx += len(marker)
    new_content = content[:idx] + insertion + content[idx:]
    with Path(filepath).open("w") as f:
        f.write(new_content)
    return True


def main():
    if len(sys.argv) != 2:
        logger.info(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = Path(sys.argv[1]).resolve()

    # 1. Hook into net_client_init
    net_c = Path(qemu) / "net" / "net.c"
    marker4 = "#ifdef CONFIG_AF_XDP\n        [NET_CLIENT_DRIVER_AF_XDP]    = net_init_af_xdp,\n#endif"
    insertion4 = "\n        [NET_CLIENT_DRIVER_VIRTMCU]     = net_init_virtmcu,"
    if patch_file(net_c, marker4, insertion4, after=True):
        logger.info(f"  patched {net_c}")

    marker5 = "int net_init_socket(const Netdev *netdev, const char *name,"
    insertion5 = "int net_init_virtmcu(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp);\n"

    clients_h = Path(qemu) / "net" / "clients.h"
    if patch_file(clients_h, marker5, insertion5, after=False):
        logger.info(f"  patched {clients_h}")

    # 2. Add virtmcu.c to net/meson.build
    meson_build = Path(qemu) / "net" / "meson.build"
    marker6 = "  'checksum.c',"
    insertion6 = "\n  'virtmcu.c',"
    if patch_file(meson_build, marker6, insertion6, after=True):
        logger.info(f"  patched {meson_build}")

    # 7. Patch util/module.c to allow re-entrant module loads gracefully
    module_c = Path(qemu) / "util" / "module.c"
    marker7 = "    assert(QTAILQ_EMPTY(&dso_init_list));"
    insertion7 = """    /* assert(QTAILQ_EMPTY(&dso_init_list)); */
    if (!QTAILQ_EMPTY(&dso_init_list)) {
        error_setg(errp, "module_load_dso: re-entrant call detected loading %s"
                   " — skipping to avoid corruption", fname);
        return false;
    }"""
    if Path(module_c).exists():
        with Path(module_c).open() as f:
            content = f.read()
        if marker7.strip() in content:
            new_content = content.replace(marker7, insertion7)
            with Path(module_c).open("w") as f:
                f.write(new_content)

    # 6. Generate net/virtmcu.c stub
    virtmcu_c = Path(qemu) / "net" / "virtmcu.c"
    virtmcu_c_content = """#include "qemu/osdep.h"
#include "net/net.h"
#include "qapi/qapi-types-net.h"
#include "clients.h"
#include "qapi/error.h"
#include "virtmcu/hooks.h"
#include "qemu/module.h"
#include "qom/object.h"

int (*virtmcu_netdev_hook)(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp) = NULL;

int net_init_virtmcu(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp)
{
    /* QEMU modules are loaded by object types. Try to load the module providing netdev */
    if (!virtmcu_netdev_hook) {
        module_load_qom("netdev", NULL);
        object_class_by_name("netdev");
    }

    if (virtmcu_netdev_hook) {
        return virtmcu_netdev_hook(netdev, name, peer, errp);
    }

    error_setg(errp, "netdev module not loaded or hook not registered");
    return -1;
}
"""
    if not Path(virtmcu_c).exists():
        with Path(virtmcu_c).open("w") as f:
            f.write(virtmcu_c_content)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()

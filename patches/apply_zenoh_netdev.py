#!/usr/bin/env python3
import os
import sys


def patch_file(filepath, marker, insertion, after=False):
    with open(filepath, "r") as f:
        content = f.read()
    if insertion in content:
        return
    idx = content.find(marker)
    if idx == -1:
        print(f"Error: Could not find marker '{marker}' in {filepath}")
        sys.exit(1)
    if after:
        idx += len(marker)
    new_content = content[:idx] + insertion + content[idx:]
    with open(filepath, "w") as f:
        f.write(new_content)


def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = os.path.abspath(sys.argv[1])

    # 1. Hook into net_client_init
    net_c = os.path.join(qemu, "net", "net.c")
    marker4 = "#ifdef CONFIG_AF_XDP\n        [NET_CLIENT_DRIVER_AF_XDP]    = net_init_af_xdp,\n#endif"
    insertion4 = "\n        [NET_CLIENT_DRIVER_ZENOH]     = net_init_zenoh,"
    patch_file(net_c, marker4, insertion4, after=True)

    marker5 = "int net_init_socket(const Netdev *netdev, const char *name,"
    insertion5 = "int net_init_zenoh(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp);\n"

    clients_h = os.path.join(qemu, "net", "clients.h")
    patch_file(clients_h, marker5, insertion5, after=False)

    # 2. Add zenoh.c to net/meson.build
    meson_build = os.path.join(qemu, "net", "meson.build")
    marker6 = "  'checksum.c',"
    insertion6 = "\n  'zenoh.c',"
    patch_file(meson_build, marker6, insertion6, after=True)

    # 6. Generate net/zenoh.c stub
    zenoh_c = os.path.join(qemu, "net", "zenoh.c")
    zenoh_c_content = """#include "qemu/osdep.h"
#include "net/net.h"
#include "qapi/qapi-types-net.h"
#include "clients.h"
#include "qapi/error.h"
#include "virtmcu/hooks.h"
#include "qemu/module.h"
#include "qom/object.h"

int (*virtmcu_zenoh_netdev_hook)(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp) = NULL;

int net_init_zenoh(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp)
{
    /* QEMU modules are loaded by object types. Try to load the module providing zenoh-netdev */
    if (!virtmcu_zenoh_netdev_hook) {
        module_load_qom("zenoh-netdev", NULL);
        /* Force class_init to run and register the hook */
        if (object_class_by_name("zenoh-netdev")) {
            /* success */
        }
    }

    if (virtmcu_zenoh_netdev_hook) {
        return virtmcu_zenoh_netdev_hook(netdev, name, peer, errp);
    }

    error_setg(errp, "zenoh-netdev module not loaded or hook not registered");
    return -1;
}
"""
    if not os.path.exists(zenoh_c):
        with open(zenoh_c, "w") as f:
            f.write(zenoh_c_content)


if __name__ == "__main__":
    main()

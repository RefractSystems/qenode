/*
 * hw/zenoh/zenoh-netdev.c — Rust-backed Multi-Node Ethernet Backend
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */
#include "qemu/osdep.h"
#include "net/net.h"
#include "net/clients.h"
#include "hw/core/sysbus.h"
#include "qapi/error.h"
#include "qapi/qapi-types-net.h"
#include "qemu/timer.h"
#include "qom/object.h"
#include "virtmcu/hooks.h"
#include "qemu/module.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohNetdevState ZenohNetdevState;

extern ZenohNetdevState *zenoh_netdev_init(NetClientState *nc, uint32_t node_id, const char *router, const char *topic);
extern void             zenoh_netdev_cleanup_rust(ZenohNetdevState *state);
extern ssize_t          zenoh_netdev_receive_rust(ZenohNetdevState *state, const uint8_t *buf, size_t size);

typedef struct ZenohNetdevQEMU {
    SysBusDevice parent_obj;
    NetClientState nc;
    ZenohNetdevState *rust_state;
} ZenohNetdevQEMU;

static ssize_t zenoh_netdev_receive(NetClientState *nc, const uint8_t *buf, size_t size)
{
    ZenohNetdevQEMU *s = container_of(nc, ZenohNetdevQEMU, nc);
    return zenoh_netdev_receive_rust(s->rust_state, buf, size);
}

static bool zenoh_netdev_can_receive(NetClientState *nc)
{
    return true;
}

static void zenoh_netdev_cleanup(NetClientState *nc)
{
    ZenohNetdevQEMU *s = container_of(nc, ZenohNetdevQEMU, nc);
    if (s->rust_state) {
        zenoh_netdev_cleanup_rust(s->rust_state);
        s->rust_state = NULL;
    }
}

static NetClientInfo net_zenoh_info = {
    .type = NET_CLIENT_DRIVER_ZENOH,
    .size = sizeof(ZenohNetdevQEMU),
    .can_receive = zenoh_netdev_can_receive,
    .receive = zenoh_netdev_receive,
    .cleanup = zenoh_netdev_cleanup,
};

static int zenoh_netdev_hook(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp)
{
    const NetdevZenohOptions *opts = &netdev->u.zenoh;
    
    NetClientState *nc = qemu_new_net_client(&net_zenoh_info, peer, "zenoh", name);
    ZenohNetdevQEMU *s = container_of(nc, ZenohNetdevQEMU, nc);
    
    uint32_t node_id = opts->node ? atoi(opts->node) : 0;
    
    s->rust_state = zenoh_netdev_init(nc, node_id, opts->router, opts->topic);
    if (!s->rust_state) {
        error_setg(errp, "Failed to initialize Rust Zenoh netdev");
        return -1;
    }
    
    return 0;
}

#define TYPE_ZENOH_NETDEV "zenoh-netdev"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohNetdevQOM, ZENOH_NETDEV)

struct ZenohNetdevQOM {
    SysBusDevice parent_obj;
};

static void zenoh_netdev_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->user_creatable = true;
    virtmcu_zenoh_netdev_hook = zenoh_netdev_hook;
}

static const TypeInfo zenoh_netdev_types[] = {
    {
        .name          = TYPE_ZENOH_NETDEV,
        .parent        = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(ZenohNetdevQOM),
        .class_init    = zenoh_netdev_class_init,
    }
};

DEFINE_TYPES(zenoh_netdev_types)
module_obj(TYPE_ZENOH_NETDEV);

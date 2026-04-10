/*
 * virtmcu mmio-socket-bridge QOM device.
 *
 * Forwards MMIO reads/writes over a Unix socket to an external process
 * (like a SystemC co-simulation adapter) to enable Path A of the Co-Simulation Bridge.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "qemu/log.h"
#include "qemu/module.h"
#include "qemu/main-loop.h"
#include "hw/core/sysbus.h"
#include "hw/core/qdev-properties.h"
#include "qapi/error.h"
#include "qom/object.h"
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

/*
 * Wire protocol — shared with tools/systemc_adapter/main.cpp via the header
 * hw/misc/virtmcu_proto.h.  Include it directly since both files live in the
 * same hw/misc/ directory in the virtmcu repo.
 */
#include "virtmcu_proto.h"

#define TYPE_MMIO_SOCKET_BRIDGE "mmio-socket-bridge"
OBJECT_DECLARE_SIMPLE_TYPE(MmioSocketBridgeState, MMIO_SOCKET_BRIDGE)

struct MmioSocketBridgeState {
    SysBusDevice parent_obj;

    MemoryRegion mmio;

    /* Properties */
    char *socket_path;
    uint32_t region_size;
    uint64_t base_addr;

    /* Socket state */
    int sock_fd;
};

static void send_req_and_wait(MmioSocketBridgeState *s, struct mmio_req *req, struct mmio_resp *resp)
{
    if (s->sock_fd < 0) {
        return;
    }

    /*
     * Release the BQL (Big QEMU Lock) before *both* the write and the read.
     * Any blocking syscall while holding BQL deadlocks the main loop thread,
     * preventing QMP, GDB, and device I/O from being serviced.
     *
     * write() on a Unix domain socket can block if the kernel send buffer is
     * full (backpressure from a slow adapter), so it must also be outside the
     * locked window.  Re-acquire immediately after read() completes.
     */
    bql_unlock();
    ssize_t wr = write(s->sock_fd, req, sizeof(*req));
    ssize_t rd = (wr == (ssize_t)sizeof(*req))
                 ? read(s->sock_fd, resp, sizeof(*resp))
                 : -1;
    bql_lock();

    if (wr != (ssize_t)sizeof(*req)) {
        qemu_log_mask(LOG_GUEST_ERROR, "mmio-socket-bridge: socket write failed\n");
        return;
    }
    if (rd != (ssize_t)sizeof(*resp)) {
        qemu_log_mask(LOG_GUEST_ERROR, "mmio-socket-bridge: socket read failed\n");
    }
}

static uint64_t bridge_read(void *opaque, hwaddr addr, unsigned size)
{
    MmioSocketBridgeState *s = opaque;
    struct mmio_req req = {
        .type = MMIO_REQ_READ,
        .size = size,
        .addr = addr,
        .data = 0,
    };
    struct mmio_resp resp = {0};

    send_req_and_wait(s, &req, &resp);
    return resp.data;
}

static void bridge_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    MmioSocketBridgeState *s = opaque;
    struct mmio_req req = {
        .type = MMIO_REQ_WRITE,
        .size = size,
        .addr = addr,
        .data = val,
    };
    struct mmio_resp resp = {0};

    send_req_and_wait(s, &req, &resp);
}

static const MemoryRegionOps bridge_mmio_ops = {
    .read  = bridge_read,
    .write = bridge_write,
    .impl  = {
        .min_access_size = 1,
        .max_access_size = 8,
    },
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static void bridge_realize(DeviceState *dev, Error **errp)
{
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(dev);

    if (!s->socket_path) {
        error_setg(errp, "socket-path property must be set");
        return;
    }

    if (s->region_size == 0) {
        error_setg(errp, "region-size property must be > 0");
        return;
    }

    s->sock_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (s->sock_fd < 0) {
        error_setg_errno(errp, errno, "failed to create unix socket");
        return;
    }

    struct sockaddr_un addr = {0};
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, s->socket_path, sizeof(addr.sun_path) - 1);

    if (connect(s->sock_fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        error_setg_errno(errp, errno, "failed to connect to %s", s->socket_path);
        close(s->sock_fd);
        s->sock_fd = -1;
        return;
    }

    memory_region_init_io(&s->mmio, OBJECT(s), &bridge_mmio_ops, s,
                          TYPE_MMIO_SOCKET_BRIDGE, s->region_size);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->mmio);

    if (s->base_addr != UINT64_MAX) {
        sysbus_mmio_map(SYS_BUS_DEVICE(s), 0, s->base_addr);
    }
}

static void bridge_unrealize(DeviceState *dev)
{
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(dev);
    if (s->sock_fd >= 0) {
        close(s->sock_fd);
        s->sock_fd = -1;
    }
}

static const Property bridge_properties[] = {
    DEFINE_PROP_STRING("socket-path", MmioSocketBridgeState, socket_path),
    DEFINE_PROP_UINT32("region-size", MmioSocketBridgeState, region_size, 0x1000),
    DEFINE_PROP_UINT64("base-addr", MmioSocketBridgeState, base_addr, UINT64_MAX),
};

static void bridge_class_init(ObjectClass *oc, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(oc);
    
    dc->realize = bridge_realize;
    dc->unrealize = bridge_unrealize;
    device_class_set_props(dc, bridge_properties);
    dc->user_creatable = true;
}

static const TypeInfo bridge_types[] = {
    {
        .name          = TYPE_MMIO_SOCKET_BRIDGE,
        .parent        = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(MmioSocketBridgeState),
        .class_init    = bridge_class_init,
    },
};

DEFINE_TYPES(bridge_types)

module_obj(TYPE_MMIO_SOCKET_BRIDGE);

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


/* 
 * Protocol:
 * Request (from QEMU to external process):
 *  uint8_t type;  // 0=read, 1=write
 *  uint8_t size;  // access size in bytes (1, 2, 4, 8)
 *  uint16_t reserved1;
 *  uint32_t reserved2;
 *  uint64_t addr; // offset within region
 *  uint64_t data; // data to write (0 for read)
 * 
 * Response (from external process to QEMU):
 *  uint64_t data; // data read (for read), or 0/status for write
 */
struct mmio_req {
    uint8_t type;
    uint8_t size;
    uint16_t reserved1;
    uint32_t reserved2;
    uint64_t addr;
    uint64_t data;
} __attribute__((packed));

struct mmio_resp {
    uint64_t data;
} __attribute__((packed));

static void send_req_and_wait(MmioSocketBridgeState *s, struct mmio_req *req, struct mmio_resp *resp)
{
    if (s->sock_fd < 0) {
        return;
    }

    if (write(s->sock_fd, req, sizeof(*req)) != sizeof(*req)) {
        qemu_log_mask(LOG_GUEST_ERROR, "mmio-socket-bridge: socket write failed\n");
        return;
    }

    /* 
     * ADR-007: Must release BQL before any blocking system call in the vCPU thread
     * so we don't deadlock the main loop. 
     */
    bql_unlock();
    ssize_t ret = read(s->sock_fd, resp, sizeof(*resp));
    bql_lock();

    if (ret != sizeof(*resp)) {
        qemu_log_mask(LOG_GUEST_ERROR, "mmio-socket-bridge: socket read failed\n");
    }
}

static uint64_t bridge_read(void *opaque, hwaddr addr, unsigned size)
{
    MmioSocketBridgeState *s = opaque;
    struct mmio_req req = {
        .type = 0,
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
        .type = 1,
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

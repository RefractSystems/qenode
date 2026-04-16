/*
 * virtmcu mmio-socket-bridge QOM device.
 *
 * Forwards MMIO reads/writes over a Unix socket as relative offsets.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "qemu/log.h"
#include "qemu/module.h"
#include "qemu/main-loop.h"
#include "qemu/sockets.h"
#include "hw/core/sysbus.h"
#include "hw/core/qdev-properties.h"
#include "hw/core/irq.h"
#include "qapi/error.h"
#include "qom/object.h"
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <errno.h>

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
    QemuMutex sock_mutex;
    QemuCond resp_cond;
    bool has_resp;
    struct sysc_msg current_resp;
    uint8_t rx_buf[16];
    int rx_idx;
    qemu_irq irqs[32];
};

static bool writen(int fd, const void *buf, size_t len)
{
    const char *p = buf;
    while (len > 0) {
        ssize_t n = write(fd, p, len);
        if (n <= 0) {
            if (n < 0 && (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)) continue;
            return false;
        }
        p += n; len -= n;
    }
    return true;
}

static void bridge_sock_handler(void *opaque)
{
    MmioSocketBridgeState *s = opaque;
    while (1) {
        int n = read(s->sock_fd, s->rx_buf + s->rx_idx, sizeof(struct sysc_msg) - s->rx_idx);
        if (n <= 0) {
            if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR)) return;
            qemu_set_fd_handler(s->sock_fd, NULL, NULL, NULL);
            close(s->sock_fd); s->sock_fd = -1;
            qemu_mutex_lock(&s->sock_mutex);
            s->has_resp = true; qemu_cond_broadcast(&s->resp_cond);
            qemu_mutex_unlock(&s->sock_mutex);
            return;
        }
        s->rx_idx += n;
        if (s->rx_idx == sizeof(struct sysc_msg)) {
            struct sysc_msg *msg = (struct sysc_msg *)s->rx_buf;
            if (msg->type == SYSC_MSG_IRQ_SET && msg->irq_num < 32) qemu_set_irq(s->irqs[msg->irq_num], 1);
            else if (msg->type == SYSC_MSG_IRQ_CLEAR && msg->irq_num < 32) qemu_set_irq(s->irqs[msg->irq_num], 0);
            else if (msg->type == SYSC_MSG_RESP) {
                qemu_mutex_lock(&s->sock_mutex);
                s->current_resp = *msg; s->has_resp = true;
                qemu_cond_broadcast(&s->resp_cond);
                qemu_mutex_unlock(&s->sock_mutex);
            }
            s->rx_idx = 0;
        }
    }
}

static void send_req_and_wait(MmioSocketBridgeState *s, struct mmio_req *req, struct sysc_msg *resp)
{
    if (s->sock_fd < 0) return;
    bql_unlock();
    qemu_mutex_lock(&s->sock_mutex);
    s->has_resp = false;
    if (writen(s->sock_fd, req, sizeof(*req))) {
        while (!s->has_resp) qemu_cond_wait(&s->resp_cond, &s->sock_mutex);
        *resp = s->current_resp;
    }
    qemu_mutex_unlock(&s->sock_mutex);
    bql_lock();
}

static uint64_t bridge_read(void *opaque, hwaddr addr, unsigned size)
{
    MmioSocketBridgeState *s = opaque;
    struct mmio_req req = {
        .type = MMIO_REQ_READ, .size = size,
        .vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL),
        .addr = addr, .data = 0,
    };
    struct sysc_msg resp = {0};
    send_req_and_wait(s, &req, &resp);
    return resp.data;
}

static void bridge_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    MmioSocketBridgeState *s = opaque;
    struct mmio_req req = {
        .type = MMIO_REQ_WRITE, .size = size,
        .vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL),
        .addr = addr, .data = val,
    };
    struct sysc_msg resp = {0};
    send_req_and_wait(s, &req, &resp);
}

static const MemoryRegionOps bridge_mmio_ops = {
    .read = bridge_read, .write = bridge_write,
    .impl = { .min_access_size = 1, .max_access_size = 8 },
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static void bridge_realize(DeviceState *dev, Error **errp)
{
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(dev);
    if (!s->socket_path) { error_setg(errp, "socket-path must be set"); return; }
    if (s->region_size == 0) { error_setg(errp, "region-size must be > 0"); return; }
    for (int i = 0; i < 32; i++) sysbus_init_irq(SYS_BUS_DEVICE(dev), &s->irqs[i]);
    s->sock_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    struct sockaddr_un addr = { .sun_family = AF_UNIX };
    strncpy(addr.sun_path, s->socket_path, sizeof(addr.sun_path) - 1);
    if (connect(s->sock_fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        error_setg_errno(errp, errno, "failed to connect to %s", s->socket_path);
        close(s->sock_fd); s->sock_fd = -1; return;
    }
    g_unix_set_fd_nonblocking(s->sock_fd, true, NULL);
    qemu_set_fd_handler(s->sock_fd, bridge_sock_handler, NULL, s);
    memory_region_init_io(&s->mmio, OBJECT(s), &bridge_mmio_ops, s, "mmio-socket-bridge", s->region_size);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->mmio);
    if (s->base_addr != UINT64_MAX) {
        sysbus_mmio_map(SYS_BUS_DEVICE(s), 0, s->base_addr);
    }
}

static void bridge_instance_init(Object *obj) {
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(obj);
    s->sock_fd = -1; qemu_mutex_init(&s->sock_mutex); qemu_cond_init(&s->resp_cond);
}
static void bridge_instance_finalize(Object *obj) {
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(obj);
    qemu_mutex_destroy(&s->sock_mutex); qemu_cond_destroy(&s->resp_cond);
}
static void bridge_unrealize(DeviceState *dev) {
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(dev);
    if (s->sock_fd >= 0) { qemu_set_fd_handler(s->sock_fd, NULL, NULL, NULL); close(s->sock_fd); s->sock_fd = -1; }
}
static const Property bridge_properties[] = {
    DEFINE_PROP_STRING("socket-path", MmioSocketBridgeState, socket_path),
    DEFINE_PROP_UINT32("region-size", MmioSocketBridgeState, region_size, 0),
    DEFINE_PROP_UINT64("base-addr", MmioSocketBridgeState, base_addr, UINT64_MAX),
};
static void bridge_class_init(ObjectClass *klass, const void *data) {
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = bridge_realize; dc->unrealize = bridge_unrealize;
    device_class_set_props(dc, bridge_properties); dc->user_creatable = true;
}
static const TypeInfo bridge_types[] = {
    { .name = TYPE_MMIO_SOCKET_BRIDGE, .parent = TYPE_SYS_BUS_DEVICE,
      .instance_size = sizeof(MmioSocketBridgeState), .instance_init = bridge_instance_init,
      .instance_finalize = bridge_instance_finalize, .class_init = bridge_class_init },
};
DEFINE_TYPES(bridge_types)
module_obj(TYPE_MMIO_SOCKET_BRIDGE);

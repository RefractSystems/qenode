/*
 * virtmcu mmio-socket-bridge QOM device.
 *
 * Forwards MMIO reads/writes over a Unix socket as relative offsets.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "qemu/log.h"
#include "qemu/error-report.h"
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
#include <poll.h>

#include "virtmcu_proto.h"

#define TYPE_MMIO_SOCKET_BRIDGE "mmio-socket-bridge"

/* Maximum time to wait for a response from the SystemC adapter per MMIO op. */
#define BRIDGE_TIMEOUT_MS 500
OBJECT_DECLARE_SIMPLE_TYPE(MmioSocketBridgeState, MMIO_SOCKET_BRIDGE)

struct MmioSocketBridgeState {
    SysBusDevice parent_obj;
    MemoryRegion mmio;

    /* Properties */
    char *socket_path;
    uint32_t region_size;
    uint64_t base_addr;
    uint32_t reconnect_ms;

    /* Socket state */
    int sock_fd;
    QemuMutex sock_mutex;
    QemuCond resp_cond;
    bool has_resp;
    bool resp_valid;
    union {
        struct sysc_msg msg;
        uint64_t align;
    } current_resp;
    union {
        struct sysc_msg msg;
        uint8_t bytes[sizeof(struct sysc_msg)];
        uint64_t align;
    } rx_buf;
    int rx_idx;
    qemu_irq irqs[32];
    QEMUTimer *reconnect_timer;
};

static void bridge_connect(MmioSocketBridgeState *s);

static void reconnect_timer_cb(void *opaque)
{
    MmioSocketBridgeState *s = (MmioSocketBridgeState *)opaque;
    bridge_connect(s);
}

static bool writen(int fd, const void *buf, size_t len)
{
    const char *p = (const char *)buf;
    struct pollfd pfd = { .fd = fd, .events = POLLOUT };
    while (len > 0) {
        int ret = poll(&pfd, 1, BRIDGE_TIMEOUT_MS);
        if (ret <= 0) {
            if (ret < 0 && errno == EINTR) continue;
            return false;
        }
        ssize_t n = write(fd, p, len);
        if (n <= 0) {
            if (n < 0 && (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)) continue;
            return false;
        }
        p += n; len -= n;
    }
    return true;
}

static int read_timeout(int fd, void *buf, size_t len, int timeout_ms)
{
    struct pollfd pfd = { .fd = fd, .events = POLLIN };
    int ret = poll(&pfd, 1, timeout_ms);
    if (ret <= 0) return ret;
    return read(fd, buf, len);
}

static void bridge_sock_handler(void *opaque)
{
    MmioSocketBridgeState *s = (MmioSocketBridgeState *)opaque;
    while (1) {
        int n = read(s->sock_fd, s->rx_buf.bytes + s->rx_idx, sizeof(struct sysc_msg) - s->rx_idx);
        if (n <= 0) {
            if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR)) return;
            fprintf(stderr, "mmio-socket-bridge: remote disconnected, closing socket fd %d\n", s->sock_fd);
            qemu_set_fd_handler(s->sock_fd, NULL, NULL, NULL);
            close(s->sock_fd); s->sock_fd = -1;
            
            qemu_mutex_lock(&s->sock_mutex);
            s->has_resp = true; s->resp_valid = false;
            qemu_cond_broadcast(&s->resp_cond);
            qemu_mutex_unlock(&s->sock_mutex);

            if (s->reconnect_ms > 0) {
                timer_mod(s->reconnect_timer, qemu_clock_get_ms(QEMU_CLOCK_REALTIME) + s->reconnect_ms);
            }
            return;
        }
        s->rx_idx += n;
        if (s->rx_idx == sizeof(struct sysc_msg)) {
            struct sysc_msg *msg = &s->rx_buf.msg;
            if (msg->type == SYSC_MSG_IRQ_SET || msg->type == SYSC_MSG_IRQ_CLEAR) {
                if (msg->irq_num < 32) {
                    bool locked = bql_locked();
                    if (!locked) bql_lock();
                    qemu_set_irq(s->irqs[msg->irq_num], msg->type == SYSC_MSG_IRQ_SET);
                    if (!locked) bql_unlock();
                }
            } else if (msg->type == SYSC_MSG_RESP) {
                qemu_mutex_lock(&s->sock_mutex);
                memcpy(&s->current_resp.msg, msg, sizeof(struct sysc_msg));
                s->has_resp = true; s->resp_valid = true;
                qemu_cond_broadcast(&s->resp_cond);
                qemu_mutex_unlock(&s->sock_mutex);
            }
            s->rx_idx = 0;
        }
    }
}

static void bridge_connect(MmioSocketBridgeState *s)
{
    if (s->sock_fd >= 0) return;

    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return;

    struct sockaddr_un addr = { .sun_family = AF_UNIX };
    strncpy(addr.sun_path, s->socket_path, sizeof(addr.sun_path) - 1);
    
    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        if (s->reconnect_ms > 0) {
            timer_mod(s->reconnect_timer, qemu_clock_get_ms(QEMU_CLOCK_REALTIME) + s->reconnect_ms);
        }
        return;
    }

    struct virtmcu_handshake hs_out = { VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION };
    if (!writen(fd, &hs_out, sizeof(hs_out))) { close(fd); return; }

    struct virtmcu_handshake hs_in;
    int n = read_timeout(fd, &hs_in, sizeof(hs_in), BRIDGE_TIMEOUT_MS);
    if (n != sizeof(hs_in) || hs_in.magic != VIRTMCU_PROTO_MAGIC || hs_in.version != VIRTMCU_PROTO_VERSION) {
        close(fd); return;
    }

    s->sock_fd = fd;
    s->rx_idx = 0;
    g_unix_set_fd_nonblocking(s->sock_fd, true, NULL);
    qemu_set_fd_handler(s->sock_fd, bridge_sock_handler, NULL, s);
    fprintf(stderr, "mmio-socket-bridge: connected to %s\n", s->socket_path);
}

static void send_req_and_wait(MmioSocketBridgeState *s, struct mmio_req *req, struct sysc_msg *resp)
{
    if (s->sock_fd < 0) return;
    bql_unlock();
    qemu_mutex_lock(&s->sock_mutex);
    s->has_resp = false;
    if (writen(s->sock_fd, req, sizeof(*req))) {
        bool timed_out = false;
        while (!s->has_resp) {
            if (!qemu_cond_timedwait(&s->resp_cond, &s->sock_mutex, BRIDGE_TIMEOUT_MS)) {
                timed_out = true;
                break;
            }
        }
        if (timed_out) {
            int fd = s->sock_fd;
            s->sock_fd = -1;
            qemu_mutex_unlock(&s->sock_mutex);
            bql_lock();
            fprintf(stderr, "mmio-socket-bridge: timeout on socket fd %d after %d ms, disconnecting\n",
                fd, BRIDGE_TIMEOUT_MS);
            qemu_set_fd_handler(fd, NULL, NULL, NULL);
            close(fd);
            return;
        }
        if (s->resp_valid) {
            memcpy(resp, &s->current_resp.msg, sizeof(struct sysc_msg));
        }
    }
    qemu_mutex_unlock(&s->sock_mutex);
    bql_lock();
}

static uint64_t bridge_read(void *opaque, hwaddr addr, unsigned size)
{
    MmioSocketBridgeState *s = (MmioSocketBridgeState *)opaque;
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
    MmioSocketBridgeState *s = (MmioSocketBridgeState *)opaque;
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

    s->reconnect_timer = timer_new_ms(QEMU_CLOCK_REALTIME, reconnect_timer_cb, s);
    bridge_connect(s);

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
    if (s->reconnect_timer) {
        timer_free(s->reconnect_timer);
    }
    qemu_mutex_destroy(&s->sock_mutex); qemu_cond_destroy(&s->resp_cond);
}
static void bridge_unrealize(DeviceState *dev) {
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(dev);
    if (s->reconnect_timer) {
        timer_del(s->reconnect_timer);
    }
    if (s->sock_fd >= 0) { qemu_set_fd_handler(s->sock_fd, NULL, NULL, NULL); close(s->sock_fd); s->sock_fd = -1; }
}
static const Property bridge_properties[] = {
    DEFINE_PROP_STRING("socket-path", MmioSocketBridgeState, socket_path),
    DEFINE_PROP_UINT32("region-size", MmioSocketBridgeState, region_size, 0),
    DEFINE_PROP_UINT64("base-addr", MmioSocketBridgeState, base_addr, UINT64_MAX),
    DEFINE_PROP_UINT32("reconnect-ms", MmioSocketBridgeState, reconnect_ms, 0),
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

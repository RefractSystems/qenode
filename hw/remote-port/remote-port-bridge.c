/*
 * virtmcu remote-port-bridge QOM device.
 *
 * Implements AMD/Xilinx Remote Port to communicate with Verilator/SystemC.
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

#pragma GCC diagnostic ignored "-Wdeprecated-declarations"
#include "remote-port-proto.h"

#define TYPE_REMOTE_PORT_BRIDGE "remote-port-bridge"
OBJECT_DECLARE_SIMPLE_TYPE(RemotePortBridgeState, REMOTE_PORT_BRIDGE)

struct RemotePortBridgeState {
    SysBusDevice parent_obj;

    MemoryRegion mmio;

    char *socket_path;
    uint32_t region_size;
    uint64_t base_addr;

    int sock_fd;
    QemuMutex sock_mutex;
    QemuCond resp_cond;
    bool has_resp;

    struct rp_pkt current_resp;
    uint8_t current_data[8];

    uint8_t rx_buf[4096];
    int rx_idx;
    int expected_len;

    qemu_irq irqs[32];
    uint32_t next_id;
};

static bool writen(int fd, const void *buf, size_t len)
{
    const char *p = buf;
    while (len > 0) {
        ssize_t n = write(fd, p, len);
        if (n <= 0) {
            if (n < 0 && (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)) {
                continue;
            }
            return false;
        }
        p   += n;
        len -= n;
    }
    return true;
}

static bool readn(int fd, void *buf, size_t len)
{
    char *p = buf;
    while (len > 0) {
        ssize_t n = read(fd, p, len);
        if (n <= 0) {
            if (n < 0 && (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK)) {
                usleep(1000);
                continue;
            }
            return false;
        }
        p   += n;
        len -= n;
    }
    return true;
}

static void handle_rx_packet(RemotePortBridgeState *s, struct rp_pkt *pkt, uint8_t *data)
{
    rp_decode_payload(pkt);
    
    if (pkt->hdr.cmd == RP_CMD_interrupt) {
        uint32_t line = pkt->interrupt.line;
        uint8_t val = pkt->interrupt.val;
        if (line < 32) {
            qemu_set_irq(s->irqs[line], val ? 1 : 0);
        }
    } else {
        qemu_mutex_lock(&s->sock_mutex);
        s->current_resp = *pkt;
        if (pkt->hdr.cmd == RP_CMD_read && data && pkt->hdr.len >= sizeof(pkt->busaccess) - sizeof(pkt->hdr)) {
            uint32_t data_len = pkt->hdr.len - (sizeof(pkt->busaccess) - sizeof(pkt->hdr));
            if (data_len <= 8) {
                memcpy(s->current_data, data, data_len);
            }
        }
        s->has_resp = true;
        qemu_cond_broadcast(&s->resp_cond);
        qemu_mutex_unlock(&s->sock_mutex);
    }
}

static void bridge_sock_handler(void *opaque)
{
    RemotePortBridgeState *s = opaque;
    
    while (1) {
        if (s->expected_len == 0 && s->rx_idx >= sizeof(struct rp_pkt_hdr)) {
            struct rp_pkt *pkt = (struct rp_pkt *)s->rx_buf;
            struct rp_pkt_hdr hdr = pkt->hdr;
            hdr.len = be32toh(hdr.len);
            s->expected_len = sizeof(struct rp_pkt_hdr) + hdr.len;
        }
        
        int to_read = (s->expected_len > 0) ? (s->expected_len - s->rx_idx) : (sizeof(struct rp_pkt_hdr) - s->rx_idx);
        if (to_read <= 0 && s->expected_len > 0) {
            struct rp_pkt pkt;
            memcpy(&pkt, s->rx_buf, MIN(sizeof(pkt), s->expected_len));
            rp_decode_hdr(&pkt);
            
            uint8_t *data = NULL;
            if (pkt.hdr.cmd == RP_CMD_read && s->expected_len > sizeof(struct rp_pkt_busaccess)) {
                data = s->rx_buf + sizeof(struct rp_pkt_busaccess);
            }
            handle_rx_packet(s, &pkt, data);
            
            if (s->rx_idx > s->expected_len) {
                memmove(s->rx_buf, s->rx_buf + s->expected_len, s->rx_idx - s->expected_len);
                s->rx_idx -= s->expected_len;
            } else {
                s->rx_idx = 0;
            }
            s->expected_len = 0;
            continue;
        }
        
        int n = read(s->sock_fd, s->rx_buf + s->rx_idx, to_read);
        if (n <= 0) {
            if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR)) {
                return;
            }
            qemu_set_fd_handler(s->sock_fd, NULL, NULL, NULL);
            close(s->sock_fd);
            s->sock_fd = -1;
            
            qemu_mutex_lock(&s->sock_mutex);
            s->has_resp = true;
            qemu_cond_broadcast(&s->resp_cond);
            qemu_mutex_unlock(&s->sock_mutex);
            return;
        }
        s->rx_idx += n;
    }
}

static void send_req_and_wait(RemotePortBridgeState *s, struct rp_pkt *req, size_t req_len, uint8_t *req_data, struct rp_pkt *resp, uint8_t *resp_data)
{
    if (s->sock_fd < 0) return;

    bql_unlock();
    qemu_mutex_lock(&s->sock_mutex);
    s->has_resp = false;

    bool ok = writen(s->sock_fd, req, req_len);
    if (ok && req_data) {
        uint32_t payload_len = be32toh(req->hdr.len);
        uint32_t to_write = payload_len - (req_len - sizeof(struct rp_pkt_hdr));
        ok = writen(s->sock_fd, req_data, to_write);
    }

    if (ok) {
        while (!s->has_resp) {
            qemu_cond_wait(&s->resp_cond, &s->sock_mutex);
        }
        *resp = s->current_resp;
        if (resp_data && resp->hdr.cmd == RP_CMD_read) {
            memcpy(resp_data, s->current_data, 8);
        }
    }

    qemu_mutex_unlock(&s->sock_mutex);
    bql_lock();
}

static uint64_t bridge_read(void *opaque, hwaddr addr, unsigned size)
{
    RemotePortBridgeState *s = opaque;
    struct rp_pkt req = {0};
    struct rp_pkt resp = {0};
    uint8_t data[8] = {0};
    
    size_t len = rp_encode_read(s->next_id++, 0, &req.busaccess, 0, 0, addr, 0, size, size, size);
    send_req_and_wait(s, &req, len, NULL, &resp, data);

    uint64_t val = 0;
    if (size == 1) val = data[0];
    else if (size == 2) val = *(uint16_t*)data;
    else if (size == 4) val = *(uint32_t*)data;
    else if (size == 8) val = *(uint64_t*)data;
    return val;
}

static void bridge_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    RemotePortBridgeState *s = opaque;
    struct rp_pkt req = {0};
    struct rp_pkt resp = {0};
    uint8_t data[8] = {0};
    
    if (size == 1) data[0] = val;
    else if (size == 2) *(uint16_t*)data = val;
    else if (size == 4) *(uint32_t*)data = val;
    else if (size == 8) *(uint64_t*)data = val;
    
    size_t len = rp_encode_write(s->next_id++, 0, &req.busaccess, 0, 0, addr, 0, size, size, size);
    send_req_and_wait(s, &req, len, data, &resp, NULL);
}

static const MemoryRegionOps bridge_ops = {
    .read = bridge_read,
    .write = bridge_write,
    .endianness = DEVICE_NATIVE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
    .impl = { .min_access_size = 1, .max_access_size = 8 },
};

static void bridge_realize(DeviceState *dev, Error **errp)
{
    RemotePortBridgeState *s = REMOTE_PORT_BRIDGE(dev);

    if (!s->socket_path) {
        error_setg(errp, "socket-path property must be set");
        return;
    }
    if (s->region_size == 0) {
        error_setg(errp, "region-size property must be set > 0");
        return;
    }

    struct sockaddr_un un;
    if (strlen(s->socket_path) >= sizeof(un.sun_path)) {
        error_setg(errp, "socket-path is too long");
        return;
    }

    s->sock_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (s->sock_fd < 0) {
        error_setg_errno(errp, errno, "Failed to create socket");
        return;
    }

    un.sun_family = AF_UNIX;
    strcpy(un.sun_path, s->socket_path);

    if (connect(s->sock_fd, (struct sockaddr *)&un, sizeof(un)) < 0) {
        error_setg_errno(errp, errno, "Failed to connect to %s", s->socket_path);
        close(s->sock_fd);
        s->sock_fd = -1;
        return;
    }

    g_unix_set_fd_nonblocking(s->sock_fd, true, NULL);
    
    // Remote Port HELLO handshake
    struct rp_pkt hello_req = {0};
    struct rp_pkt hello_resp = {0};
    size_t len = rp_encode_hello(0, 0, &hello_req.hello, RP_VERSION_MAJOR, RP_VERSION_MINOR);
    
    if (!writen(s->sock_fd, &hello_req, len)) {
        error_setg(errp, "Failed to send Remote port HELLO");
        close(s->sock_fd);
        s->sock_fd = -1;
        return;
    }
    
    if (!readn(s->sock_fd, &hello_resp.hdr, sizeof(hello_resp.hdr))) {
        error_setg_errno(errp, errno, "Failed to read Remote port HELLO response header");
        close(s->sock_fd);
        s->sock_fd = -1;
        return;
    }
    
    hello_resp.hdr.len = be32toh(hello_resp.hdr.len);
    if (hello_resp.hdr.len > 0) {
        if (!readn(s->sock_fd, (uint8_t *)&hello_resp + sizeof(struct rp_pkt_hdr), hello_resp.hdr.len)) {
            error_setg_errno(errp, errno, "Failed to read Remote port HELLO response body");
            close(s->sock_fd);
            s->sock_fd = -1;
            return;
        }
    }
    hello_resp.hdr.cmd = be32toh(hello_resp.hdr.cmd);
    
    if (hello_resp.hdr.cmd != RP_CMD_hello) {
        error_setg(errp, "Remote port HELLO handshake failed (got %d, expected %d)", hello_resp.hdr.cmd, RP_CMD_hello);
        close(s->sock_fd);
        s->sock_fd = -1;
        return;
    }

    qemu_set_fd_handler(s->sock_fd, bridge_sock_handler, NULL, s);

    memory_region_init_io(&s->mmio, OBJECT(s), &bridge_ops, s,
                          "remote-port-bridge", s->region_size);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->mmio);

    if (s->base_addr != 0) {
        sysbus_mmio_map(SYS_BUS_DEVICE(s), 0, s->base_addr);
    }
}

static void bridge_init(Object *obj)
{
    RemotePortBridgeState *s = REMOTE_PORT_BRIDGE(obj);
    qemu_mutex_init(&s->sock_mutex);
    qemu_cond_init(&s->resp_cond);
    s->sock_fd = -1;

    for (int i = 0; i < 32; i++) {
        sysbus_init_irq(SYS_BUS_DEVICE(s), &s->irqs[i]);
    }
}

static const Property bridge_properties[] = {
    DEFINE_PROP_STRING("socket-path", RemotePortBridgeState, socket_path),
    DEFINE_PROP_UINT32("region-size", RemotePortBridgeState, region_size, 0x1000),
    DEFINE_PROP_UINT64("base-addr", RemotePortBridgeState, base_addr, 0),
};

static void bridge_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = bridge_realize;
    device_class_set_props(dc, bridge_properties);
}

static const TypeInfo bridge_types[] = {
    {
        .name          = TYPE_REMOTE_PORT_BRIDGE,
        .parent        = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(RemotePortBridgeState),
        .instance_init = bridge_init,
        .class_init    = bridge_class_init,
    }
};

DEFINE_TYPES(bridge_types)

module_obj(TYPE_REMOTE_PORT_BRIDGE);

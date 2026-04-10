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
#include <errno.h>

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

    /*
     * Serialises socket I/O independently of the BQL.
     *
     * Why: the correct BQL discipline for blocking I/O is:
     *   bql_unlock() → write() → read() → bql_lock()
     * This means a second vCPU can acquire the BQL while the first is blocked
     * in the socket write/read.  If both vCPUs then enter send_req_and_wait()
     * they would call write() on the same sock_fd concurrently — Unix domain
     * socket writes are not atomic above the pipe buffer size, so the 24-byte
     * request messages would interleave and corrupt the framing protocol.
     *
     * sock_mutex is held only across the write+read pair (inside the BQL-free
     * window) so it does not reintroduce the BQL deadlock risk.
     */
    QemuMutex sock_mutex;
};

/*
 * Write exactly @len bytes to @fd, retrying on short writes.
 * Returns true on success, false on error/EOF.
 * Must be called outside the BQL window.
 */
static bool writen(int fd, const void *buf, size_t len)
{
    const char *p = buf;
    while (len > 0) {
        ssize_t n = write(fd, p, len);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) {
                continue;
            }
            return false;
        }
        p   += n;
        len -= n;
    }
    return true;
}

/*
 * Read exactly @len bytes from @fd, retrying on short reads.
 * Returns true on success, false on error/EOF.
 * Must be called outside the BQL window.
 */
static bool readn(int fd, void *buf, size_t len)
{
    char *p = buf;
    while (len > 0) {
        ssize_t n = read(fd, p, len);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) {
                continue;
            }
            return false;
        }
        p   += n;
        len -= n;
    }
    return true;
}

static void send_req_and_wait(MmioSocketBridgeState *s, struct mmio_req *req, struct mmio_resp *resp)
{
    if (s->sock_fd < 0) {
        return;
    }

    /*
     * Release the BQL before *both* the write and the read, then take
     * sock_mutex to serialise concurrent vCPU accesses on the socket.
     *
     * BQL discipline: any blocking syscall while holding BQL deadlocks the
     * main loop (QMP, GDB, I/O).  write() can block under backpressure.
     *
     * SMP safety: after bql_unlock() a second vCPU can enter this function.
     * sock_mutex ensures only one write+read pair is in flight at a time;
     * otherwise the 24-byte request frames would interleave on the socket and
     * corrupt the protocol framing.
     */
    bql_unlock();
    qemu_mutex_lock(&s->sock_mutex);

    bool ok = writen(s->sock_fd, req, sizeof(*req));
    if (ok) {
        ok = readn(s->sock_fd, resp, sizeof(*resp));
    }

    qemu_mutex_unlock(&s->sock_mutex);
    bql_lock();

    if (!ok) {
        qemu_log_mask(LOG_GUEST_ERROR,
                      "mmio-socket-bridge: I/O error on socket '%s'\n",
                      s->socket_path);
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

static void bridge_instance_init(Object *obj)
{
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(obj);
    s->sock_fd = -1;
    qemu_mutex_init(&s->sock_mutex);
}

static void bridge_instance_finalize(Object *obj)
{
    MmioSocketBridgeState *s = MMIO_SOCKET_BRIDGE(obj);
    qemu_mutex_destroy(&s->sock_mutex);
}

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
        .name            = TYPE_MMIO_SOCKET_BRIDGE,
        .parent          = TYPE_SYS_BUS_DEVICE,
        .instance_size   = sizeof(MmioSocketBridgeState),
        .instance_init   = bridge_instance_init,
        .instance_finalize = bridge_instance_finalize,
        .class_init      = bridge_class_init,
    },
};

DEFINE_TYPES(bridge_types)

module_obj(TYPE_MMIO_SOCKET_BRIDGE);

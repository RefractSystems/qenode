/*
 * hw/zenoh/zenoh-netdev.c — Deterministic Multi-Node Ethernet Backend
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 *
 * This file implements a custom QEMU `-netdev` backend that uses Zenoh as the
 * transport layer instead of traditional UDP/TCP sockets. This is critical for
 * deterministic multi-node simulation.
 *
 * How it works:
 * 1. On TX: When the guest NIC sends an Ethernet frame, this backend prefixes
 *    it with a `ZenohFrameHeader` containing the exact `QEMU_CLOCK_VIRTUAL`
 *    time. The frame is then published to `sim/eth/frame/{src_node}/tx`.
 * 2. On RX: A Zenoh subscriber listens for incoming frames. Instead of
 *    injecting them immediately (which would introduce wall-clock jitter), it
 *    reads the `delivery_vtime_ns` timestamp from the header. It then arms a
 *    QEMUTimer to fire at exactly that virtual time. Only when the virtual
 *    clock reaches the delivery time is the frame injected into the guest NIC
 *    via `qemu_send_packet`.
 *
 * This guarantees that network events are causally ordered by virtual time
 * across all nodes, regardless of host scheduling or network latency.
 */
#include "qemu/osdep.h"
#include "net/net.h"
#include "net/clients.h"
#include "qapi/error.h"
#include "qapi/qapi-types-net.h"
#include "qemu/timer.h"
#include "qemu/main-loop.h"
#include "qom/object.h"
#include "virtmcu/hooks.h"
#include "qemu/module.h"
#include <zenoh.h>

typedef struct ZenohNetdevState {
    NetClientState nc;
    z_owned_session_t session;
    z_owned_publisher_t publisher;
    z_owned_subscriber_t subscriber;
    QEMUTimer *rx_timer;
    uint32_t node_id;
    char *router;
    /* Basic queue implementation for demo purposes */
    struct {
        uint64_t delivery_vtime;
        uint8_t *data;
        size_t size;
    } rx_queue[64];
    int rx_count;
    QemuMutex mutex;
} ZenohNetdevState;

typedef struct __attribute__((packed)) {
    uint64_t delivery_vtime_ns;
    uint32_t size;
    /* payload follows */
} ZenohFrameHeader;

static void push_rx_frame(ZenohNetdevState *s, uint64_t vtime, const uint8_t *data, size_t size)
{
    qemu_mutex_lock(&s->mutex);
    if (s->rx_count < 64) {
        /*
         * Insertion sort into rx_queue.
         * The queue is sorted in descending order (highest vtime at index 0,
         * lowest vtime at index rx_count-1) to allow O(1) popping from the end.
         */
        int i = s->rx_count - 1;
        while (i >= 0 && s->rx_queue[i].delivery_vtime < vtime) {
            s->rx_queue[i + 1] = s->rx_queue[i];
            i--;
        }
        s->rx_queue[i + 1].delivery_vtime = vtime;
        s->rx_queue[i + 1].data = g_memdup2(data, size);
        s->rx_queue[i + 1].size = size;
        s->rx_count++;
        
        timer_mod(s->rx_timer, s->rx_queue[s->rx_count - 1].delivery_vtime);
    } else {
        /* Drop frame if queue is full */
    }
    qemu_mutex_unlock(&s->mutex);
}

static void on_rx_frame(z_loaned_sample_t *sample, void *context)
{
    ZenohNetdevState *s = context;
    const z_loaned_bytes_t *payload = z_sample_payload(sample);
    if (!payload) return;
    
    z_bytes_reader_t reader = z_bytes_get_reader(payload);
    ZenohFrameHeader hdr;
    if (z_bytes_reader_read(&reader, (uint8_t*)&hdr, sizeof(hdr)) != sizeof(hdr)) {
        return;
    }
    
    uint8_t *frame_data = g_malloc(hdr.size);
    if (z_bytes_reader_read(&reader, frame_data, hdr.size) == hdr.size) {
        push_rx_frame(s, hdr.delivery_vtime_ns, frame_data, hdr.size);
    }
    g_free(frame_data);
}

static void rx_timer_cb(void *opaque)
{
    ZenohNetdevState *s = opaque;
    qemu_mutex_lock(&s->mutex);
    
    uint64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    while (s->rx_count > 0) {
        int last = s->rx_count - 1;
        if (s->rx_queue[last].delivery_vtime <= now) {
            uint8_t *data = s->rx_queue[last].data;
            size_t size = s->rx_queue[last].size;
            s->rx_count--;
            
            qemu_mutex_unlock(&s->mutex);
            fprintf(stderr, "zenoh-netdev: sending RX packet to guest, size=%zu\n", size);
            qemu_send_packet(&s->nc, data, size);
            g_free(data);
            qemu_mutex_lock(&s->mutex);
        } else {
            timer_mod(s->rx_timer, s->rx_queue[last].delivery_vtime);
            break;
        }
    }
    qemu_mutex_unlock(&s->mutex);
}

static ssize_t zenoh_netdev_receive(NetClientState *nc, const uint8_t *buf, size_t size)
{
    ZenohNetdevState *s = DO_UPCAST(ZenohNetdevState, nc, nc);
    
    ZenohFrameHeader hdr = {
        .delivery_vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL),
        .size = size,
    };
    
    uint8_t *msg = g_malloc(sizeof(hdr) + size);
    memcpy(msg, &hdr, sizeof(hdr));
    memcpy(msg + sizeof(hdr), buf, size);
    
    z_owned_bytes_t payload;
    z_bytes_copy_from_buf(&payload, msg, sizeof(hdr) + size);
    
    z_publisher_put(z_publisher_loan(&s->publisher), z_move(payload), NULL);
    
    g_free(msg);
    return size;
}

static bool zenoh_netdev_can_receive(NetClientState *nc)
{
    return true;
}

static void zenoh_netdev_cleanup(NetClientState *nc)
{
    ZenohNetdevState *s = DO_UPCAST(ZenohNetdevState, nc, nc);
    
    if (s->rx_timer) {
        timer_free(s->rx_timer);
        s->rx_timer = NULL;
    }
    z_publisher_drop(z_move(s->publisher));
    z_subscriber_drop(z_move(s->subscriber));
    z_close(z_session_loan_mut(&s->session), NULL);
    z_session_drop(z_move(s->session));
}

static NetClientInfo net_zenoh_info = {
    .type = NET_CLIENT_DRIVER_ZENOH,
    .size = sizeof(ZenohNetdevState),
    .can_receive = zenoh_netdev_can_receive,
    .receive = zenoh_netdev_receive,
    .cleanup = zenoh_netdev_cleanup,
};

static int zenoh_netdev_hook(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp)
{
    const NetdevZenohOptions *zenoh_opts = &netdev->u.zenoh;
    
    NetClientState *nc = qemu_new_net_client(&net_zenoh_info, peer, "zenoh", name);
    ZenohNetdevState *s = DO_UPCAST(ZenohNetdevState, nc, nc);
    
    s->node_id = atoi(zenoh_opts->node);
    
    qemu_mutex_init(&s->mutex);
    s->rx_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, s);
    
    z_owned_config_t config;
    z_config_default(&config);
    if (zenoh_opts->router) {
        char json[256];
        snprintf(json, sizeof(json), "[\"%s\"]", zenoh_opts->router);
        zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", json);
    }
    if (z_open(&s->session, z_move(config), NULL) != 0) {
        error_setg(errp, "Failed to open Zenoh session for netdev");
        return -1;
    }
    
    char topic_tx[128];
    snprintf(topic_tx, sizeof(topic_tx), "sim/eth/frame/%u/tx", s->node_id);
    
    z_owned_keyexpr_t kexpr_tx;
    z_keyexpr_from_str(&kexpr_tx, topic_tx);
    z_declare_publisher(z_session_loan(&s->session), &s->publisher, z_keyexpr_loan(&kexpr_tx), NULL);
    z_keyexpr_drop(z_move(kexpr_tx));
    
    char topic_rx[128];
    snprintf(topic_rx, sizeof(topic_rx), "sim/eth/frame/%u/rx", s->node_id);
    
    z_owned_closure_sample_t callback;
    z_closure_sample(&callback, on_rx_frame, NULL, s);
    
    z_owned_keyexpr_t kexpr_rx;
    z_keyexpr_from_str(&kexpr_rx, topic_rx);
    z_declare_subscriber(z_session_loan(&s->session), &s->subscriber, z_keyexpr_loan(&kexpr_rx), z_move(callback), NULL);
    z_keyexpr_drop(z_move(kexpr_rx));
    
    return 0;
}

#define TYPE_ZENOH_NETDEV "zenoh-netdev"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohNetdevQOM, ZENOH_NETDEV)

struct ZenohNetdevQOM {
    Object parent_obj;
};

static void zenoh_netdev_class_init(ObjectClass *klass, const void *data)
{
    /* Register hook */
    virtmcu_zenoh_netdev_hook = zenoh_netdev_hook;
}

static const TypeInfo zenoh_netdev_types[] = {
    {
        .name          = TYPE_ZENOH_NETDEV,
        .parent        = TYPE_OBJECT,
        .instance_size = sizeof(ZenohNetdevQOM),
        .class_init    = zenoh_netdev_class_init,
    }
};

DEFINE_TYPES(zenoh_netdev_types)
module_obj(TYPE_ZENOH_NETDEV);

/*
 * hw/zenoh/zenoh-802154.c — Deterministic 802.15.4 Radio Peripheral
 *
 * This device models a generic 802.15.4 radio (similar to an nRF24 or
 * Atmel AT86RF233) that uses Zenoh for transport.
 *
 * Registers:
 * 0x00: TX_DATA (W) — Write bytes to fill the TX FIFO.
 * 0x04: TX_LEN  (RW) — Length of the packet to send.
 * 0x08: TX_GO   (W) — Write any value to trigger transmission.
 * 0x0C: RX_DATA (R) — Read bytes from the RX FIFO.
 * 0x10: RX_LEN  (R) — Length of the received packet.
 * 0x14: STATUS  (R) — Bit 0: RX_READY, Bit 1: TX_DONE.
 * 0x18: RSSI    (R) — RSSI of the last received packet.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "qapi/error.h"
#include "qemu/timer.h"
#include "qemu/main-loop.h"
#include "qom/object.h"
#include "qemu/module.h"
#include "qemu/error-report.h"
#include "hw/core/qdev-properties.h"
#include "hw/core/irq.h"
#include <zenoh.h>

#define TYPE_ZENOH_802154 "zenoh-802154"
OBJECT_DECLARE_SIMPLE_TYPE(Zenoh802154State, ZENOH_802154)

typedef struct __attribute__((packed)) {
    uint64_t delivery_vtime_ns;
    uint32_t size;
    int8_t rssi;
    uint8_t lqi;
} ZenohRfHeader;

struct Zenoh802154State {
    SysBusDevice parent_obj;
    MemoryRegion iomem;
    qemu_irq irq;

    /* Properties */
    uint32_t node_id;
    char *router;
    char *topic;

    /* Zenoh state */
    z_owned_session_t session;
    z_owned_publisher_t publisher;
    z_owned_subscriber_t subscriber;

    /* Hardware state */
    uint8_t tx_fifo[128];
    uint32_t tx_len;
    uint8_t rx_fifo[128];
    uint32_t rx_len;
    int8_t rx_rssi;
    uint32_t status;

    /* Timing */
    QEMUTimer *rx_timer;
    struct {
        uint64_t delivery_vtime;
        uint8_t data[128];
        size_t size;
        int8_t rssi;
    } rx_queue[16];
    int rx_count;
    QemuMutex mutex;
};

static void push_rx_frame(Zenoh802154State *s, uint64_t vtime, const uint8_t *data, size_t size, int8_t rssi)
{
    qemu_mutex_lock(&s->mutex);
    if (s->rx_count < 16) {
        int i = s->rx_count - 1;
        while (i >= 0 && s->rx_queue[i].delivery_vtime < vtime) {
            s->rx_queue[i + 1] = s->rx_queue[i];
            i--;
        }
        s->rx_queue[i + 1].delivery_vtime = vtime;
        memcpy(s->rx_queue[i + 1].data, data, size);
        s->rx_queue[i + 1].size = size;
        s->rx_queue[i + 1].rssi = rssi;
        s->rx_count++;
        
        timer_mod(s->rx_timer, s->rx_queue[s->rx_count - 1].delivery_vtime);
    }
    qemu_mutex_unlock(&s->mutex);
}

static void on_rx_frame(z_loaned_sample_t *sample, void *context)
{
    Zenoh802154State *s = context;
    const z_loaned_bytes_t *payload = z_sample_payload(sample);
    if (!payload) return;
    
    z_bytes_reader_t reader = z_bytes_get_reader(payload);
    ZenohRfHeader hdr;
    if (z_bytes_reader_read(&reader, (uint8_t*)&hdr, sizeof(hdr)) != sizeof(hdr)) {
        return;
    }
    
    uint8_t frame_data[128];
    if (hdr.size <= 128 && z_bytes_reader_read(&reader, frame_data, hdr.size) == hdr.size) {
        push_rx_frame(s, hdr.delivery_vtime_ns, frame_data, hdr.size, hdr.rssi);
    }
}

static void rx_timer_cb(void *opaque)
{
    Zenoh802154State *s = opaque;
    qemu_mutex_lock(&s->mutex);
    
    uint64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    if (s->rx_count > 0) {
        int last = s->rx_count - 1;
        if (s->rx_queue[last].delivery_vtime <= now) {
            memcpy(s->rx_fifo, s->rx_queue[last].data, s->rx_queue[last].size);
            s->rx_len = s->rx_queue[last].size;
            s->rx_rssi = s->rx_queue[last].rssi;
            s->rx_count--;
            
            s->status |= 0x01; /* RX_READY */
            qemu_set_irq(s->irq, 1);
            
            if (s->rx_count > 0) {
                timer_mod(s->rx_timer, s->rx_queue[s->rx_count - 1].delivery_vtime);
            }
        } else {
            timer_mod(s->rx_timer, s->rx_queue[last].delivery_vtime);
        }
    }
    qemu_mutex_unlock(&s->mutex);
}

static uint64_t zenoh_802154_read(void *opaque, hwaddr offset, unsigned size)
{
    Zenoh802154State *s = opaque;
    uint32_t val = 0;

    switch (offset) {
    case 0x04: val = s->tx_len; break;
    case 0x0C:
        if (s->rx_len > 0) {
            val = s->rx_fifo[0]; /* Simple FIFO read - not a true FIFO in this mock */
        }
        break;
    case 0x10: val = s->rx_len; break;
    case 0x14: val = s->status; break;
    case 0x18: val = (uint8_t)s->rx_rssi; break;
    }
    return val;
}

static void zenoh_802154_write(void *opaque, hwaddr offset, uint64_t value, unsigned size)
{
    Zenoh802154State *s = opaque;

    switch (offset) {
    case 0x00:
        if (s->tx_len < 128) {
            s->tx_fifo[s->tx_len++] = (uint8_t)value;
        }
        break;
    case 0x04:
        s->tx_len = value & 0x7F;
        break;
    case 0x08:
        /* TX GO */
        {
            ZenohRfHeader hdr = {
                .delivery_vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL),
                .size = s->tx_len,
                .rssi = 0, /* Filled by coordinator */
                .lqi = 255
            };
            uint8_t msg[sizeof(hdr) + 128];
            memcpy(msg, &hdr, sizeof(hdr));
            memcpy(msg + sizeof(hdr), s->tx_fifo, s->tx_len);
            
            z_owned_bytes_t payload;
            z_bytes_copy_from_buf(&payload, msg, sizeof(hdr) + s->tx_len);
            z_publisher_put(z_publisher_loan(&s->publisher), z_move(payload), NULL);
            
            s->tx_len = 0;
            s->status |= 0x02; /* TX_DONE */
            qemu_set_irq(s->irq, 1);
        }
        break;
    case 0x14:
        s->status &= ~value;
        if (s->status == 0) {
            qemu_set_irq(s->irq, 0);
        }
        break;
    }
}

static const MemoryRegionOps zenoh_802154_ops = {
    .read = zenoh_802154_read,
    .write = zenoh_802154_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static void zenoh_802154_realize(DeviceState *dev, Error **errp)
{
    Zenoh802154State *s = ZENOH_802154(dev);

    z_owned_config_t config;
    z_config_default(&config);
    if (s->router) {
        char json[256];
        snprintf(json, sizeof(json), "[\"%s\"]", s->router);
        zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", json);
        zc_config_insert_json5(z_config_loan_mut(&config), "scouting/multicast/enabled", "false");
    }

    if (z_open(&s->session, z_move(config), NULL) != 0) {
        error_setg(errp, "Failed to open Zenoh session for 802.15.4");
        return;
    }

    char topic_tx[256], topic_rx[256];
    if (s->topic) {
        snprintf(topic_tx, sizeof(topic_tx), "%s/tx", s->topic);
        snprintf(topic_rx, sizeof(topic_rx), "%s/rx", s->topic);
    } else {
        snprintf(topic_tx, sizeof(topic_tx), "sim/rf/802154/%u/tx", s->node_id);
        snprintf(topic_rx, sizeof(topic_rx), "sim/rf/802154/%u/rx", s->node_id);
    }

    z_owned_keyexpr_t kexpr_tx;
    z_keyexpr_from_str(&kexpr_tx, topic_tx);
    z_declare_publisher(z_session_loan(&s->session), &s->publisher, z_keyexpr_loan(&kexpr_tx), NULL);
    z_keyexpr_drop(z_move(kexpr_tx));

    z_owned_closure_sample_t callback;
    z_closure_sample(&callback, on_rx_frame, NULL, s);
    z_owned_keyexpr_t kexpr_rx;
    z_keyexpr_from_str(&kexpr_rx, topic_rx);
    z_declare_subscriber(z_session_loan(&s->session), &s->subscriber, z_keyexpr_loan(&kexpr_rx), z_move(callback), NULL);
    z_keyexpr_drop(z_move(kexpr_rx));

    qemu_mutex_init(&s->mutex);
    s->rx_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, s);
}

static void zenoh_802154_init(Object *obj)
{
    Zenoh802154State *s = ZENOH_802154(obj);
    SysBusDevice *sbd = SYS_BUS_DEVICE(obj);

    memory_region_init_io(&s->iomem, obj, &zenoh_802154_ops, s, TYPE_ZENOH_802154, 0x100);
    sysbus_init_mmio(sbd, &s->iomem);
    sysbus_init_irq(sbd, &s->irq);
}

static const Property zenoh_802154_properties[] = {
    DEFINE_PROP_UINT32("node",   Zenoh802154State, node_id, 0),
    DEFINE_PROP_STRING("router", Zenoh802154State, router),
    DEFINE_PROP_STRING("topic",  Zenoh802154State, topic),
};

static void zenoh_802154_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_802154_realize;
    device_class_set_props(dc, zenoh_802154_properties);
}

static const TypeInfo zenoh_802154_info = {
    .name          = TYPE_ZENOH_802154,
    .parent        = TYPE_SYS_BUS_DEVICE,
    .instance_size = sizeof(Zenoh802154State),
    .instance_init = zenoh_802154_init,
    .class_init    = zenoh_802154_class_init,
};

static void zenoh_802154_register_types(void)
{
    type_register_static(&zenoh_802154_info);
}

type_init(zenoh_802154_register_types)

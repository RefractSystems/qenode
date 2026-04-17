/*
 * hw/zenoh/zenoh-802154.c — Rust-backed Deterministic 802.15.4 Radio
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "qapi/error.h"
#include "qemu/timer.h"
#include "qom/object.h"
#include "qemu/module.h"
#include "hw/core/qdev-properties.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct Zenoh802154State Zenoh802154State;

extern Zenoh802154State *zenoh_802154_init_rust(qemu_irq irq, uint32_t node_id, const char *router, const char *topic);
extern uint32_t          zenoh_802154_read_rust(Zenoh802154State *state, uint64_t offset);
extern void              zenoh_802154_write_rust(Zenoh802154State *state, uint64_t offset, uint64_t value);
extern void              zenoh_802154_cleanup_rust(Zenoh802154State *state);

#define TYPE_ZENOH_802154 "zenoh-802154"
OBJECT_DECLARE_SIMPLE_TYPE(Zenoh802154QEMU, ZENOH_802154)

struct Zenoh802154QEMU {
    SysBusDevice parent_obj;
    MemoryRegion iomem;
    qemu_irq irq;

    /* Properties */
    uint32_t node_id;
    char *router;
    char *topic;

    /* Rust state */
    Zenoh802154State *rust_state;
};

static uint64_t zenoh_802154_read(void *opaque, hwaddr offset, unsigned size)
{
    Zenoh802154QEMU *s = opaque;
    return zenoh_802154_read_rust(s->rust_state, offset);
}

static void zenoh_802154_write(void *opaque, hwaddr offset, uint64_t value, unsigned size)
{
    Zenoh802154QEMU *s = opaque;
    zenoh_802154_write_rust(s->rust_state, offset, value);
}

static const MemoryRegionOps zenoh_802154_ops = {
    .read = zenoh_802154_read,
    .write = zenoh_802154_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static void zenoh_802154_realize(DeviceState *dev, Error **errp)
{
    Zenoh802154QEMU *s = ZENOH_802154(dev);

    s->rust_state = zenoh_802154_init_rust(s->irq, s->node_id, s->router, s->topic);
    if (!s->rust_state) {
        error_setg(errp, "Failed to initialize Rust Zenoh 802.15.4");
        return;
    }
}

static void zenoh_802154_finalize(Object *obj)
{
    Zenoh802154QEMU *s = ZENOH_802154(obj);
    if (s->rust_state) {
        zenoh_802154_cleanup_rust(s->rust_state);
        s->rust_state = NULL;
    }
}

static void zenoh_802154_init(Object *obj)
{
    Zenoh802154QEMU *s = ZENOH_802154(obj);
    SysBusDevice *sbd = SYS_BUS_DEVICE(obj);

    memory_region_init_io(&s->iomem, obj, &zenoh_802154_ops, s, TYPE_ZENOH_802154, 0x100);
    sysbus_init_mmio(sbd, &s->iomem);
    sysbus_init_irq(sbd, &s->irq);
}

static const Property zenoh_802154_properties[] = {
    DEFINE_PROP_UINT32("node",   Zenoh802154QEMU, node_id, 0),
    DEFINE_PROP_STRING("router", Zenoh802154QEMU, router),
    DEFINE_PROP_STRING("topic",  Zenoh802154QEMU, topic),
};

static void zenoh_802154_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_802154_realize;
    device_class_set_props(dc, zenoh_802154_properties);
    dc->user_creatable = true;
}

static const TypeInfo zenoh_802154_types[] = {
    {
        .name          = TYPE_ZENOH_802154,
        .parent        = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(Zenoh802154QEMU),
        .instance_init = zenoh_802154_init,
        .instance_finalize = zenoh_802154_finalize,
        .class_init    = zenoh_802154_class_init,
    }
};

DEFINE_TYPES(zenoh_802154_types)
module_obj(TYPE_ZENOH_802154);

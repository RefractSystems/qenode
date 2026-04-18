/*
 * hw/zenoh/zenoh-actuator.c — Rust-backed Zenoh Actuator / Control Device
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "qapi/error.h"
#include "qemu/module.h"
#include "qemu/timer.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohActuatorState ZenohActuatorState;

extern ZenohActuatorState *zenoh_actuator_init_rust(uint32_t node_id, const char *router, const char *topic_prefix);
extern void                zenoh_actuator_publish_rust(ZenohActuatorState *state, uint32_t actuator_id, uint32_t data_size, const double *data);
extern void                zenoh_actuator_cleanup_rust(ZenohActuatorState *state);

#define TYPE_ZENOH_ACTUATOR "zenoh-actuator"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohActuatorQEMU, ZENOH_ACTUATOR)

#define REG_ACTUATOR_ID 0x00
#define REG_DATA_SIZE   0x04
#define REG_GO          0x08
#define REG_DATA_START  0x10

struct ZenohActuatorQEMU {
    SysBusDevice parent_obj;

    MemoryRegion mmio;

    /* Properties */
    uint32_t node_id;
    char    *router;
    char    *topic_prefix;

    /* Registers */
    uint32_t actuator_id;
    uint32_t data_size;
    double   data[8];

    /* Rust state */
    ZenohActuatorState *rust_state;
};

static uint64_t zenoh_actuator_read(void *opaque, hwaddr addr, unsigned size)
{
    ZenohActuatorQEMU *s = opaque;

    if (addr == REG_ACTUATOR_ID) {
        return s->actuator_id;
    } else if (addr == REG_DATA_SIZE) {
        return s->data_size;
    } else if (addr >= REG_DATA_START && addr < REG_DATA_START + 8 * 8) {
        int idx = (addr - REG_DATA_START) / 8;
        int offset = (addr - REG_DATA_START) % 8;
        uint64_t ret = 0;
        if (offset + size <= 8) {
            memcpy(&ret, (uint8_t *)&s->data[idx] + offset, size);
        }
        return ret;
    }

    return 0;
}

static void zenoh_actuator_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    ZenohActuatorQEMU *s = opaque;

    if (addr == REG_ACTUATOR_ID) {
        s->actuator_id = (uint32_t)val;
    } else if (addr == REG_DATA_SIZE) {
        s->data_size = (uint32_t)val;
        if (s->data_size > 8) s->data_size = 8;
    } else if (addr == REG_GO) {
        if (val == 1) {
            zenoh_actuator_publish_rust(s->rust_state, s->actuator_id, s->data_size, s->data);
        }
    } else if (addr >= REG_DATA_START && addr < REG_DATA_START + 8 * 8) {
        int idx = (addr - REG_DATA_START) / 8;
        int offset = (addr - REG_DATA_START) % 8;
        if (offset + size <= 8) {
            memcpy((uint8_t *)&s->data[idx] + offset, &val, size);
        }
    }
}

static const MemoryRegionOps zenoh_actuator_ops = {
    .read = zenoh_actuator_read,
    .write = zenoh_actuator_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = {
        .min_access_size = 4,
        .max_access_size = 8,
    },
};

static void zenoh_actuator_realize(DeviceState *dev, Error **errp)
{
    ZenohActuatorQEMU *s = ZENOH_ACTUATOR(dev);

    memory_region_init_io(&s->mmio, OBJECT(s), &zenoh_actuator_ops, s,
                          TYPE_ZENOH_ACTUATOR, 0x100);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->mmio);

    s->rust_state = zenoh_actuator_init_rust(s->node_id, s->router, s->topic_prefix);
    if (!s->rust_state) {
        error_setg(errp, "zenoh-actuator: failed to initialize Rust backend (check Zenoh router/connectivity)");
        return;
    }
}

static void zenoh_actuator_finalize(Object *obj)
{
    ZenohActuatorQEMU *s = ZENOH_ACTUATOR(obj);
    if (s->rust_state) {
        zenoh_actuator_cleanup_rust(s->rust_state);
        s->rust_state = NULL;
    }
}

static const Property zenoh_actuator_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohActuatorQEMU, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohActuatorQEMU, router),
    DEFINE_PROP_STRING("topic-prefix", ZenohActuatorQEMU, topic_prefix),
};

static void zenoh_actuator_init(Object *obj)
{
    ZenohActuatorQEMU *s = ZENOH_ACTUATOR(obj);
    s->topic_prefix = g_strdup("firmware/control");
}

static void zenoh_actuator_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_actuator_realize;
    device_class_set_props(dc, zenoh_actuator_properties);
    dc->user_creatable = true;
}

static const TypeInfo zenoh_actuator_types[] = {
    {
        .name = TYPE_ZENOH_ACTUATOR,
        .parent = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(ZenohActuatorQEMU),
        .instance_init = zenoh_actuator_init,
        .instance_finalize = zenoh_actuator_finalize,
        .class_init = zenoh_actuator_class_init,
    },
};

DEFINE_TYPES(zenoh_actuator_types)
module_obj(TYPE_ZENOH_ACTUATOR);

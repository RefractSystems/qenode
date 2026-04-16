/*
 * hw/zenoh/zenoh-actuator.c — Native Zenoh Actuator / Control Device
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 *
 * This device allows firmware to publish control signals directly to Zenoh
 * topics. It maps MMIO registers that the firmware can write to.
 *
 * Registers:
 *   0x00: ACTUATOR_ID (RW, uint32)
 *   0x04: DATA_SIZE   (RW, uint32) - Number of doubles in DATA (1-8)
 *   0x08: GO          (WO, uint32) - Write 1 to publish
 *   0x10: DATA[0..7]  (RW, double) - Up to 8 double values
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "qapi/error.h"
#include "qemu/log.h"
#include "qemu/module.h"
#include "qemu/timer.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"
#include <zenoh.h>

#define TYPE_ZENOH_ACTUATOR "zenoh-actuator"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohActuatorState, ZENOH_ACTUATOR)

#define REG_ACTUATOR_ID 0x00
#define REG_DATA_SIZE   0x04
#define REG_GO          0x08
#define REG_DATA_START  0x10

struct ZenohActuatorState {
    SysBusDevice parent_obj;

    MemoryRegion mmio;

    /* Properties */
    uint32_t node_id;
    char    *router;
    char    *topic_prefix; /* Default: "firmware/control" */

    /* Registers */
    uint32_t actuator_id;
    uint32_t data_size;
    double   data[8];

    /* Zenoh state */
    z_owned_session_t session;
    GHashTable *publishers; /* key: actuator_id -> z_owned_publisher_t* */
};

static z_owned_publisher_t *get_publisher(ZenohActuatorState *s, uint32_t act_id)
{
    z_owned_publisher_t *pub = g_hash_table_lookup(s->publishers, GUINT_TO_POINTER(act_id));
    if (pub) {
        return pub;
    }

    char topic[256];
    snprintf(topic, sizeof(topic), "%s/%u/%u", s->topic_prefix, s->node_id, act_id);

    z_owned_keyexpr_t ke;
    z_keyexpr_from_str(&ke, topic);
    
    pub = g_new0(z_owned_publisher_t, 1);
    if (z_declare_publisher(z_session_loan(&s->session), pub, z_keyexpr_loan(&ke), NULL) != 0) {
        g_free(pub);
        z_keyexpr_drop(z_move(ke));
        return NULL;
    }
    z_keyexpr_drop(z_move(ke));

    g_hash_table_insert(s->publishers, GUINT_TO_POINTER(act_id), pub);
    return pub;
}

static uint64_t zenoh_actuator_read(void *opaque, hwaddr addr, unsigned size)
{
    ZenohActuatorState *s = opaque;

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
    ZenohActuatorState *s = opaque;

    if (addr == REG_ACTUATOR_ID) {
        s->actuator_id = (uint32_t)val;
    } else if (addr == REG_DATA_SIZE) {
        s->data_size = (uint32_t)val;
        if (s->data_size > 8) s->data_size = 8;
    } else if (addr == REG_GO) {
        if (val == 1) {
            z_owned_publisher_t *pub = get_publisher(s, s->actuator_id);
            if (pub) {
                uint64_t vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
                size_t payload_size = sizeof(uint64_t) + s->data_size * sizeof(double);
                uint8_t *buf = g_malloc(payload_size);
                
                memcpy(buf, &vtime, sizeof(vtime));
                memcpy(buf + sizeof(vtime), s->data, s->data_size * sizeof(double));

                z_owned_bytes_t bytes;
                z_bytes_copy_from_buf(&bytes, buf, payload_size);
                z_publisher_put(z_publisher_loan(pub), z_move(bytes), NULL);
                
                g_free(buf);
            }
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

static void publisher_destroy(gpointer data)
{
    z_owned_publisher_t *pub = data;
    z_publisher_drop(z_move(*pub));
    g_free(pub);
}

static void zenoh_actuator_realize(DeviceState *dev, Error **errp)
{
    ZenohActuatorState *s = ZENOH_ACTUATOR(dev);

    memory_region_init_io(&s->mmio, OBJECT(s), &zenoh_actuator_ops, s,
                          TYPE_ZENOH_ACTUATOR, 0x100);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->mmio);

    s->publishers = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, publisher_destroy);

    z_owned_config_t config;
    z_config_default(&config);

    if (s->router) {
        char json[256];
        snprintf(json, sizeof(json), "[\"%s\"]", s->router);
        zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", json);
    }

    if (z_open(&s->session, z_move(config), NULL) != 0) {
        error_setg(errp, "Failed to open Zenoh session");
        return;
    }
}

static void zenoh_actuator_finalize(Object *obj)
{
    ZenohActuatorState *s = ZENOH_ACTUATOR(obj);

    if (s->publishers) {
        g_hash_table_destroy(s->publishers);
    }

    z_close(z_session_loan_mut(&s->session), NULL);
    z_session_drop(z_move(s->session));
}

static const Property zenoh_actuator_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohActuatorState, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohActuatorState, router),
    DEFINE_PROP_STRING("topic-prefix", ZenohActuatorState, topic_prefix),
};

static void zenoh_actuator_init(Object *obj)
{
    ZenohActuatorState *s = ZENOH_ACTUATOR(obj);
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
        .instance_size = sizeof(ZenohActuatorState),
        .instance_init = zenoh_actuator_init,
        .instance_finalize = zenoh_actuator_finalize,
        .class_init = zenoh_actuator_class_init,
    },
};

DEFINE_TYPES(zenoh_actuator_types)
module_obj(TYPE_ZENOH_ACTUATOR);

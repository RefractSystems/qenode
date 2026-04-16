/*
 * hw/zenoh/zenoh-ui.c — Standardized UI Topics (Buttons/LEDs)
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "hw/irq.h"
#include "qapi/error.h"
#include "qemu/log.h"
#include "qemu/module.h"
#include "qemu/timer.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"
#include "qemu/error-report.h"
#include <zenoh.h>

#define TYPE_ZENOH_UI "zenoh-ui"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohUiState, ZENOH_UI)

#define REG_LED_ID      0x00
#define REG_LED_STATE   0x04
#define REG_BTN_ID      0x10
#define REG_BTN_STATE   0x14
#define REG_BTN_CONFIG  0x18

typedef struct {
    uint32_t id;
    bool state;
    qemu_irq irq;
    z_owned_subscriber_t sub;
} ZenohButton;

struct ZenohUiState {
    SysBusDevice parent_obj;
    MemoryRegion mmio;

    /* Properties */
    uint32_t node_id;
    char    *router;

    /* Registers */
    uint32_t active_led_id;
    uint32_t active_btn_id;

    /* Zenoh state */
    z_owned_session_t session;
    GHashTable *led_publishers; /* key: led_id -> z_owned_publisher_t* */
    GHashTable *buttons;        /* key: btn_id -> ZenohButton* */
};

static void on_button_msg(z_loaned_sample_t *sample, void *context)
{
    ZenohButton *btn = context;
    const z_loaned_bytes_t *payload = z_sample_payload(sample);
    if (!payload) return;

    size_t len = z_bytes_len(payload);
    if (len >= 1) {
        uint8_t val;
        z_bytes_reader_t reader = z_bytes_get_reader(payload);
        z_bytes_reader_read(&reader, &val, 1);
        bool new_state = (val != 0);
        if (new_state != btn->state) {
            btn->state = new_state;
            if (btn->irq) {
                qemu_set_irq(btn->irq, btn->state ? 1 : 0);
            }
        }
    }
}

static z_owned_publisher_t *get_led_publisher(ZenohUiState *s, uint32_t led_id)
{
    z_owned_publisher_t *pub = g_hash_table_lookup(s->led_publishers, GUINT_TO_POINTER(led_id));
    if (pub) return pub;

    char topic[128];
    snprintf(topic, sizeof(topic), "sim/ui/%u/led/%u", s->node_id, led_id);
    z_owned_keyexpr_t ke;
    z_keyexpr_from_str(&ke, topic);
    
    pub = g_new0(z_owned_publisher_t, 1);
    if (z_declare_publisher(z_session_loan(&s->session), pub, z_keyexpr_loan(&ke), NULL) != 0) {
        g_free(pub);
        z_keyexpr_drop(z_move(ke));
        return NULL;
    }
    z_keyexpr_drop(z_move(ke));
    g_hash_table_insert(s->led_publishers, GUINT_TO_POINTER(led_id), pub);
    return pub;
}

static ZenohButton *get_button(ZenohUiState *s, uint32_t btn_id)
{
    ZenohButton *btn = g_hash_table_lookup(s->buttons, GUINT_TO_POINTER(btn_id));
    if (btn) return btn;

    btn = g_new0(ZenohButton, 1);
    btn->id = btn_id;
    
    char topic[128];
    snprintf(topic, sizeof(topic), "sim/ui/%u/button/%u", s->node_id, btn_id);
    z_owned_keyexpr_t ke;
    z_keyexpr_from_str(&ke, topic);
    
    z_owned_closure_sample_t cb;
    z_closure_sample(&cb, on_button_msg, NULL, btn);
    
    if (z_declare_subscriber(z_session_loan(&s->session), &btn->sub, z_keyexpr_loan(&ke), z_move(cb), NULL) != 0) {
        z_keyexpr_drop(z_move(ke));
        g_free(btn);
        return NULL;
    }
    z_keyexpr_drop(z_move(ke));
    g_hash_table_insert(s->buttons, GUINT_TO_POINTER(btn_id), btn);
    return btn;
}

static uint64_t zenoh_ui_read(void *opaque, hwaddr addr, unsigned size)
{
    ZenohUiState *s = opaque;
    if (addr == REG_LED_ID) return s->active_led_id;
    if (addr == REG_BTN_ID) return s->active_btn_id;
    if (addr == REG_BTN_STATE) {
        ZenohButton *btn = g_hash_table_lookup(s->buttons, GUINT_TO_POINTER(s->active_btn_id));
        return btn ? (btn->state ? 1 : 0) : 0;
    }
    return 0;
}

static void zenoh_ui_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    ZenohUiState *s = opaque;
    if (addr == REG_LED_ID) {
        s->active_led_id = (uint32_t)val;
    } else if (addr == REG_LED_STATE) {
        z_owned_publisher_t *pub = get_led_publisher(s, s->active_led_id);
        if (pub) {
            uint8_t state = (val != 0);
            z_owned_bytes_t bytes;
            z_bytes_copy_from_buf(&bytes, &state, 1);
            z_publisher_put(z_publisher_loan(pub), z_move(bytes), NULL);
        }
    } else if (addr == REG_BTN_ID) {
        s->active_btn_id = (uint32_t)val;
        get_button(s, s->active_btn_id); /* Ensure subscriber exists */
    }
}

static const MemoryRegionOps zenoh_ui_ops = {
    .read = zenoh_ui_read,
    .write = zenoh_ui_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
};

static void publisher_destroy(gpointer data)
{
    z_owned_publisher_t *pub = data;
    z_publisher_drop(z_move(*pub));
    g_free(pub);
}

static void button_destroy(gpointer data)
{
    ZenohButton *btn = data;
    z_subscriber_drop(z_move(btn->sub));
    g_free(btn);
}

static void zenoh_ui_realize(DeviceState *dev, Error **errp)
{
    ZenohUiState *s = ZENOH_UI(dev);
    memory_region_init_io(&s->mmio, OBJECT(s), &zenoh_ui_ops, s, TYPE_ZENOH_UI, 0x100);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->mmio);

    s->led_publishers = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, publisher_destroy);
    s->buttons = g_hash_table_new_full(g_direct_hash, g_direct_equal, NULL, button_destroy);

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

static void zenoh_ui_finalize(Object *obj)
{
    ZenohUiState *s = ZENOH_UI(obj);
    if (s->led_publishers) g_hash_table_destroy(s->led_publishers);
    if (s->buttons) g_hash_table_destroy(s->buttons);
    z_close(z_session_loan_mut(&s->session), NULL);
    z_session_drop(z_move(s->session));
}

static const Property zenoh_ui_properties[] = {
    DEFINE_PROP_UINT32("node", ZenohUiState, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohUiState, router),
};

static void zenoh_ui_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_ui_realize;
    device_class_set_props(dc, zenoh_ui_properties);
}

static const TypeInfo zenoh_ui_types[] = {
    {
        .name = TYPE_ZENOH_UI,
        .parent = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(ZenohUiState),
        .instance_finalize = zenoh_ui_finalize,
        .class_init = zenoh_ui_class_init,
    },
};

DEFINE_TYPES(zenoh_ui_types)
module_obj(TYPE_ZENOH_UI);

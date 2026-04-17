/*
 * hw/zenoh/zenoh-ui.c — Rust-backed standardized UI Topics (Buttons/LEDs)
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "hw/core/irq.h"
#include "qapi/error.h"
#include "qemu/module.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohUiState ZenohUiState;

extern ZenohUiState *zenoh_ui_init_rust(uint32_t node_id, const char *router);
extern void          zenoh_ui_cleanup_rust(ZenohUiState *state);
extern void          zenoh_ui_set_led_rust(ZenohUiState *state, uint32_t led_id, bool on);
extern bool          zenoh_ui_get_button_rust(ZenohUiState *state, uint32_t btn_id);
extern void          zenoh_ui_ensure_button_rust(ZenohUiState *state, uint32_t btn_id, qemu_irq irq);

#define TYPE_ZENOH_UI "zenoh-ui"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohUiQEMU, ZENOH_UI)

#define REG_LED_ID      0x00
#define REG_LED_STATE   0x04
#define REG_BTN_ID      0x10
#define REG_BTN_STATE   0x14

struct ZenohUiQEMU {
    SysBusDevice parent_obj;
    MemoryRegion mmio;

    /* Properties */
    uint32_t node_id;
    char    *router;

    /* Registers */
    uint32_t active_led_id;
    uint32_t active_btn_id;

    /* Rust state */
    ZenohUiState *rust_state;
};

static uint64_t zenoh_ui_read(void *opaque, hwaddr addr, unsigned size)
{
    ZenohUiQEMU *s = opaque;
    if (addr == REG_LED_ID) return s->active_led_id;
    if (addr == REG_BTN_ID) return s->active_btn_id;
    if (addr == REG_BTN_STATE) {
        return zenoh_ui_get_button_rust(s->rust_state, s->active_btn_id) ? 1 : 0;
    }
    return 0;
}

static void zenoh_ui_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    ZenohUiQEMU *s = opaque;
    if (addr == REG_LED_ID) {
        s->active_led_id = (uint32_t)val;
    } else if (addr == REG_LED_STATE) {
        zenoh_ui_set_led_rust(s->rust_state, s->active_led_id, val != 0);
    } else if (addr == REG_BTN_ID) {
        s->active_btn_id = (uint32_t)val;
        /* Ensure the button is subscribed. In a real system, we'd know how many
         * IRQs we have. Here we use the SysBus IRQs. */
        qemu_irq irq = sysbus_get_connected_irq(SYS_BUS_DEVICE(s), s->active_btn_id);
        zenoh_ui_ensure_button_rust(s->rust_state, s->active_btn_id, irq);
    }
}

static const MemoryRegionOps zenoh_ui_ops = {
    .read = zenoh_ui_read,
    .write = zenoh_ui_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = {
        .min_access_size = 4,
        .max_access_size = 4,
    },
};

static void zenoh_ui_realize(DeviceState *dev, Error **errp)
{
    ZenohUiQEMU *s = ZENOH_UI(dev);
    memory_region_init_io(&s->mmio, OBJECT(s), &zenoh_ui_ops, s, TYPE_ZENOH_UI, 0x100);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->mmio);

    s->rust_state = zenoh_ui_init_rust(s->node_id, s->router);
    if (!s->rust_state) {
        error_setg(errp, "Failed to initialize Rust Zenoh UI");
        return;
    }
}

static void zenoh_ui_finalize(Object *obj)
{
    ZenohUiQEMU *s = ZENOH_UI(obj);
    if (s->rust_state) {
        zenoh_ui_cleanup_rust(s->rust_state);
        s->rust_state = NULL;
    }
}

static const Property zenoh_ui_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohUiQEMU, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohUiQEMU, router),
};

static void zenoh_ui_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_ui_realize;
    device_class_set_props(dc, zenoh_ui_properties);
    dc->user_creatable = true;
}

static const TypeInfo zenoh_ui_types[] = {
    {
        .name = TYPE_ZENOH_UI,
        .parent = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(ZenohUiQEMU),
        .instance_finalize = zenoh_ui_finalize,
        .class_init = zenoh_ui_class_init,
    },
};

DEFINE_TYPES(zenoh_ui_types)
module_obj(TYPE_ZENOH_UI);

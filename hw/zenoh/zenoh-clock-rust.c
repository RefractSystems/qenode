/*
 * hw/zenoh/zenoh-clock-rust.c — Rust-backed virtual clock synchronization.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "hw/core/qdev-properties.h"
#include "qom/object.h"
#include "qapi/error.h"
#include "system/cpu-timers.h"
#include "system/cpu-timers-internal.h"
#include "exec/icount.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohClockState ZenohClockState;

extern ZenohClockState *zenoh_clock_init(uint32_t node_id, const char *router, const char *mode);
extern void             zenoh_clock_fini(ZenohClockState *state);

/* Helper for Rust to advance icount bias without exposing timers_state struct */
void virtmcu_icount_advance(int64_t delta)
{
    qatomic_set(&timers_state.qemu_icount_bias,
                qatomic_read(&timers_state.qemu_icount_bias) + delta);
}

/* ── QOM type ─────────────────────────────────────────────────────────────── */

#define TYPE_ZENOH_CLOCK_RUST "zenoh-clock-rust"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohClockRust, ZENOH_CLOCK_RUST)

struct ZenohClockRust {
    SysBusDevice parent_obj;

    /* Properties */
    uint32_t node_id;
    char    *router;
    char    *mode;

    /* Rust state */
    ZenohClockState *rust_state;
};

static void zenoh_clock_rust_realize(DeviceState *dev, Error **errp)
{
    ZenohClockRust *s = ZENOH_CLOCK_RUST(dev);

    s->rust_state = zenoh_clock_init(s->node_id, s->router, s->mode);
    if (!s->rust_state) {
        error_setg(errp, "Failed to initialize Rust ZenohClock");
        return;
    }
}

static void zenoh_clock_rust_instance_finalize(Object *obj)
{
    ZenohClockRust *s = ZENOH_CLOCK_RUST(obj);
    if (s->rust_state) {
        zenoh_clock_fini(s->rust_state);
        s->rust_state = NULL;
    }
}

static const Property zenoh_clock_rust_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohClockRust, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohClockRust, router),
    DEFINE_PROP_STRING("mode",   ZenohClockRust, mode),
};

static void zenoh_clock_rust_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_clock_rust_realize;
    device_class_set_props(dc, zenoh_clock_rust_properties);
    dc->user_creatable = true;
}

static const TypeInfo zenoh_clock_rust_types[] = {
    {
        .name              = TYPE_ZENOH_CLOCK_RUST,
        .parent            = TYPE_SYS_BUS_DEVICE,
        .instance_size     = sizeof(ZenohClockRust),
        .instance_finalize = zenoh_clock_rust_instance_finalize,
        .class_init        = zenoh_clock_rust_class_init,
    },
};

DEFINE_TYPES(zenoh_clock_rust_types)
module_obj(TYPE_ZENOH_CLOCK_RUST);

/*
 * hw/zenoh/zenoh-clock.c — Rust-backed virtual clock synchronization.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "qemu/module.h"
#include "qemu/main-loop.h"
#include "qemu/seqlock.h"
#include "hw/core/sysbus.h"
#include "hw/core/cpu.h"
#include "hw/core/qdev-properties.h"
#include "qom/object.h"
#include "qapi/error.h"
#include "system/cpu-timers.h"
#include "system/cpu-timers-internal.h"
#include "exec/icount.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohClockState ZenohClockState;

extern ZenohClockState *zenoh_clock_init(uint32_t node_id, const char *router, const char *mode,
                                         uint32_t stall_timeout_ms);
extern void             zenoh_clock_fini(ZenohClockState *state);

/* ── QOM type ─────────────────────────────────────────────────────────────── */

#define TYPE_ZENOH_CLOCK "zenoh-clock"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohClock, ZENOH_CLOCK)

struct ZenohClock {
    SysBusDevice parent_obj;

    /* Properties */
    uint32_t node_id;
    char    *router;
    char    *mode;
    uint32_t stall_timeout_ms;

    /* Rust state */
    ZenohClockState *rust_state;
};

static void zenoh_clock_realize(DeviceState *dev, Error **errp)
{
    ZenohClock *s = ZENOH_CLOCK(dev);

    /* Resolve effective stall timeout: explicit property > env var > built-in default.
     * stall_timeout_ms==0 means "not set by user"; use VIRTMCU_STALL_TIMEOUT_MS or 5 s. */
    uint32_t stall_ms = s->stall_timeout_ms;
    if (stall_ms == 0) {
        const char *env = getenv("VIRTMCU_STALL_TIMEOUT_MS");
        stall_ms = (env && *env) ? (uint32_t)strtoul(env, NULL, 10) : 5000;
        if (stall_ms == 0) {
            stall_ms = 5000;
        }
    }

    s->rust_state = zenoh_clock_init(s->node_id, s->router, s->mode, stall_ms);
    if (!s->rust_state) {
        error_setg(errp, "Failed to initialize Rust ZenohClock");
        return;
    }
}

static void zenoh_clock_instance_finalize(Object *obj)
{
    ZenohClock *s = ZENOH_CLOCK(obj);
    if (s->rust_state) {
        zenoh_clock_fini(s->rust_state);
        s->rust_state = NULL;
    }
}

static const Property zenoh_clock_properties[] = {
    DEFINE_PROP_UINT32("node",          ZenohClock, node_id,          0),
    DEFINE_PROP_STRING("router",        ZenohClock, router),
    DEFINE_PROP_STRING("mode",          ZenohClock, mode),
    DEFINE_PROP_UINT32("stall-timeout", ZenohClock, stall_timeout_ms, 0),
};

static void zenoh_clock_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_clock_realize;
    device_class_set_props(dc, zenoh_clock_properties);
    dc->user_creatable = true;
}

static const TypeInfo zenoh_clock_types[] = {
    {
        .name              = TYPE_ZENOH_CLOCK,
        .parent            = TYPE_SYS_BUS_DEVICE,
        .instance_size     = sizeof(ZenohClock),
        .instance_finalize = zenoh_clock_instance_finalize,
        .class_init        = zenoh_clock_class_init,
    },
};

DEFINE_TYPES(zenoh_clock_types)
module_obj(TYPE_ZENOH_CLOCK);

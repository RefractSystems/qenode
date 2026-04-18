/*
 * hw/zenoh/zenoh-telemetry.c — Rust-backed Deterministic telemetry tracing.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"
#include "qapi/error.h"
#include "qemu/timer.h"
#include "system/cpus.h"
#include "virtmcu/hooks.h"
#include "qemu/module.h"
#include "qemu/main-loop.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohTelemetryState ZenohTelemetryState;

extern ZenohTelemetryState *zenoh_telemetry_init(uint32_t node_id, const char *router);
extern void                 zenoh_telemetry_free(ZenohTelemetryState *state);
extern void                 zenoh_telemetry_trace_cpu(ZenohTelemetryState *state, int cpu_index, bool halted);
extern void                 zenoh_telemetry_trace_irq(ZenohTelemetryState *state, uint16_t slot, uint16_t pin, int level, const char *name);

#define MAX_IRQ_SLOTS        64
static struct { void *opaque; uint16_t slot; char *path; } irq_slots[MAX_IRQ_SLOTS];
static unsigned irq_slot_count;
static QemuMutex irq_slots_lock;

static ZenohTelemetryState *global_rust_state;

static uint16_t irq_slot_for_locked(void *opaque)
{
    for (unsigned i = 0; i < irq_slot_count; i++) {
        if (irq_slots[i].opaque == opaque) {
            return irq_slots[i].slot;
        }
    }
    if (irq_slot_count < MAX_IRQ_SLOTS) {
        irq_slots[irq_slot_count].opaque = opaque;
        irq_slots[irq_slot_count].slot   = (uint16_t)irq_slot_count;
        /* path is NULL for dynamically discovered items outside realize */
        irq_slots[irq_slot_count].path   = NULL;
        return (uint16_t)irq_slot_count++;
    }
    return 0xFFFF;
}

static int cache_irq_paths_cb(Object *obj, void *opaque)
{
    if (object_dynamic_cast(obj, TYPE_DEVICE)) {
        qemu_mutex_lock(&irq_slots_lock);
        if (irq_slot_count < MAX_IRQ_SLOTS) {
            irq_slots[irq_slot_count].opaque = obj;
            irq_slots[irq_slot_count].slot = (uint16_t)irq_slot_count;
            irq_slots[irq_slot_count].path = object_get_canonical_path(obj);
            irq_slot_count++;
        }
        qemu_mutex_unlock(&irq_slots_lock);
    }
    return 0;
}

static void telemetry_cpu_halt_cb(CPUState *cpu, bool halted)
{
    qemu_mutex_lock(&irq_slots_lock);
    if (global_rust_state) {
        zenoh_telemetry_trace_cpu(global_rust_state, cpu->cpu_index, halted);
    }
    qemu_mutex_unlock(&irq_slots_lock);
}

static void telemetry_irq_cb(void *opaque, int n, int level)
{
    const char *path = NULL;
    uint16_t slot;
    
    qemu_mutex_lock(&irq_slots_lock);
    if (!global_rust_state) {
        qemu_mutex_unlock(&irq_slots_lock);
        return;
    }

    slot = irq_slot_for_locked(opaque);
    if (slot < irq_slot_count && irq_slots[slot].opaque == opaque) {
        path = irq_slots[slot].path;
    }

    zenoh_telemetry_trace_irq(global_rust_state, slot, (uint16_t)n, level, path);
    qemu_mutex_unlock(&irq_slots_lock);
}

#define TYPE_ZENOH_TELEMETRY "zenoh-telemetry"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohTelemetryQOM, ZENOH_TELEMETRY)

struct ZenohTelemetryQOM {
    SysBusDevice parent_obj;
    uint32_t node_id;
    char    *router;
    ZenohTelemetryState *rust_state;
};

static void zenoh_telemetry_realize(DeviceState *dev, Error **errp)
{
    ZenohTelemetryQOM *s = ZENOH_TELEMETRY(dev);
    
    assert(bql_locked());

    s->rust_state = zenoh_telemetry_init(s->node_id, s->router);
    if (!s->rust_state) {
        error_setg(errp, "zenoh-telemetry: failed to initialize Rust backend (check Zenoh router/connectivity)");
        return;
    }
    
    global_rust_state = s->rust_state;

    /* Pre-cache QOM paths for IRQ sources to avoid resolving outside BQL */
    qemu_mutex_lock(&irq_slots_lock);
    for (unsigned i = 0; i < irq_slot_count; i++) {
        g_free(irq_slots[i].path);
        irq_slots[i].path = NULL;
    }
    irq_slot_count = 0;
    qemu_mutex_unlock(&irq_slots_lock);

    object_child_foreach_recursive(object_get_root(), cache_irq_paths_cb, NULL);

    virtmcu_cpu_halt_hook = telemetry_cpu_halt_cb;
    virtmcu_irq_hook = telemetry_irq_cb;
}

static void zenoh_telemetry_finalize(Object *obj)
{
    ZenohTelemetryQOM *s = ZENOH_TELEMETRY(obj);

    qemu_mutex_lock(&irq_slots_lock);
    if (s->rust_state) {
        if (global_rust_state == s->rust_state) {
            virtmcu_cpu_halt_hook = NULL;
            virtmcu_irq_hook = NULL;
            global_rust_state = NULL;
        }
        zenoh_telemetry_free(s->rust_state);
        s->rust_state = NULL;
    }
    
    for (unsigned i = 0; i < irq_slot_count; i++) {
        g_free(irq_slots[i].path);
        irq_slots[i].path = NULL;
    }
    irq_slot_count = 0;
    qemu_mutex_unlock(&irq_slots_lock);
}

static const Property zenoh_telemetry_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohTelemetryQOM, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohTelemetryQOM, router),
};

static void zenoh_telemetry_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_telemetry_realize;
    device_class_set_props(dc, zenoh_telemetry_properties);
    dc->user_creatable = true;
    qemu_mutex_init(&irq_slots_lock);
}

static const TypeInfo zenoh_telemetry_types[] = {
    {
        .name              = TYPE_ZENOH_TELEMETRY,
        .parent            = TYPE_SYS_BUS_DEVICE,
        .instance_size     = sizeof(ZenohTelemetryQOM),
        .instance_finalize = zenoh_telemetry_finalize,
        .class_init        = zenoh_telemetry_class_init,
    },
};

DEFINE_TYPES(zenoh_telemetry_types)
module_obj(TYPE_ZENOH_TELEMETRY);

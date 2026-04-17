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

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohTelemetryState ZenohTelemetryState;

extern ZenohTelemetryState *zenoh_telemetry_init(uint32_t node_id, const char *router);
extern void                 zenoh_telemetry_cleanup_rust(ZenohTelemetryState *state);
extern void                 zenoh_telemetry_cpu_halt_hook(int cpu_index, bool halted);
extern void                 zenoh_telemetry_irq_hook(uint16_t slot, uint16_t pin, int level);

#define MAX_IRQ_SLOTS        64
static struct { void *opaque; uint16_t slot; } irq_slots[MAX_IRQ_SLOTS];
static unsigned irq_slot_count;
static QemuMutex irq_slots_lock;

static uint16_t irq_slot_for(void *opaque)
{
    uint16_t slot = 0xFFFF;
    qemu_mutex_lock(&irq_slots_lock);
    for (unsigned i = 0; i < irq_slot_count; i++) {
        if (irq_slots[i].opaque == opaque) {
            slot = irq_slots[i].slot;
            goto out;
        }
    }
    if (irq_slot_count < MAX_IRQ_SLOTS) {
        irq_slots[irq_slot_count].opaque = opaque;
        irq_slots[irq_slot_count].slot   = (uint16_t)irq_slot_count;
        slot = (uint16_t)irq_slot_count++;
    }
out:
    qemu_mutex_unlock(&irq_slots_lock);
    return slot;
}

static void telemetry_cpu_halt_cb(CPUState *cpu, bool halted)
{
    zenoh_telemetry_cpu_halt_hook(cpu->cpu_index, halted);
}

static void telemetry_irq_cb(void *opaque, int n, int level)
{
    uint16_t slot = irq_slot_for(opaque);
    zenoh_telemetry_irq_hook(slot, (uint16_t)n, level);
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
    
    s->rust_state = zenoh_telemetry_init(s->node_id, s->router);
    if (!s->rust_state) {
        error_setg(errp, "Failed to initialize Rust Zenoh telemetry");
        return;
    }
    
    virtmcu_cpu_halt_hook = telemetry_cpu_halt_cb;
    virtmcu_irq_hook = telemetry_irq_cb;
}

static void zenoh_telemetry_finalize(Object *obj)
{
    ZenohTelemetryQOM *s = ZENOH_TELEMETRY(obj);
    if (s->rust_state) {
        virtmcu_cpu_halt_hook = NULL;
        virtmcu_irq_hook = NULL;
        zenoh_telemetry_cleanup_rust(s->rust_state);
        s->rust_state = NULL;
    }
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

/*
 * hw/zenoh/zenoh-telemetry.c — Deterministic telemetry tracing.
 *
 * Events are enqueued from QEMU hooks (TCG thread) and published to Zenoh
 * from a dedicated background thread, keeping the hot IRQ/halt paths
 * free of network latency.
 *
 * IRQ id encoding (uint32_t):
 *   bits 31-16 — device slot (opaque pointer mapped to 0..MAX_IRQ_SLOTS-1)
 *   bits 15-0  — pin index n
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"
#include "qapi/error.h"
#include "qemu/timer.h"
#include "qemu/bswap.h"
#include "system/cpus.h"
#include "virtmcu/hooks.h"

/* QEMU defines UINT128_MAX but not uint128_t, which breaks flatcc */
typedef __uint128_t uint128_t;
#include "telemetry_builder.h"
#include <zenoh.h>

#define TYPE_ZENOH_TELEMETRY "zenoh-telemetry"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohTelemetryState, ZENOH_TELEMETRY)

/* Maximum events buffered before drops occur. */
#define TELEMETRY_QUEUE_MAX  1024
/* Maximum distinct IRQ-owning devices tracked. */
#define MAX_IRQ_SLOTS        64

typedef enum {
    TRACE_EVENT_CPU_STATE  = 0,
    TRACE_EVENT_IRQ        = 1,
    TRACE_EVENT_PERIPHERAL = 2,
} TraceEventType;

typedef struct {
    uint64_t timestamp_ns;
    uint8_t  type;
    uint32_t id;
    uint32_t value;
    char    *device_name;
} TraceEvent;

/* Device slot registry — maps opaque owner pointer to a stable 16-bit slot. */
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

struct ZenohTelemetryState {
    SysBusDevice parent_obj;

    /* Properties */
    uint32_t node_id;
    char    *router;

    /* Zenoh handles */
    z_owned_session_t   session;
    z_owned_publisher_t publisher;

    /* Async publish pipeline */
    GAsyncQueue *event_queue;
    GThread     *publish_thread;

    /* Internal state */
    bool session_open;
    bool last_halted[32]; /* Support up to 32 cores */
};

static ZenohTelemetryState *global_telemetry;

/* Called from the background publish thread only. */
static gpointer telemetry_publish_thread(gpointer arg)
{
    ZenohTelemetryState *s = arg;

    flatcc_builder_t builder;
    flatcc_builder_init(&builder);

    while (1) {
        TraceEvent *ev = g_async_queue_pop(s->event_queue);
        if (!ev) break; /* NULL sentinel — time to exit */
        
        flatcc_builder_reset(&builder);
        Virtmcu_Telemetry_TraceEvent_start_as_root(&builder);
        Virtmcu_Telemetry_TraceEvent_timestamp_ns_add(&builder, ev->timestamp_ns);
        Virtmcu_Telemetry_TraceEvent_type_add(&builder, ev->type);
        Virtmcu_Telemetry_TraceEvent_id_add(&builder, ev->id);
        Virtmcu_Telemetry_TraceEvent_value_add(&builder, ev->value);
        if (ev->device_name) {
            Virtmcu_Telemetry_TraceEvent_device_name_create_str(&builder, ev->device_name);
        }
        Virtmcu_Telemetry_TraceEvent_end_as_root(&builder);

        size_t size;
        void *buf = flatcc_builder_get_direct_buffer(&builder, &size);
        if (buf) {
            z_owned_bytes_t bytes;
            z_bytes_copy_from_buf(&bytes, buf, size);
            z_publisher_put(z_publisher_loan(&s->publisher), z_move(bytes), NULL);
        }
        g_free(ev->device_name);
        g_free(ev);
    }
    
    flatcc_builder_clear(&builder);
    return NULL;
}

/* Called from QEMU hooks (TCG thread). Enqueues; never blocks. */
static void send_event(ZenohTelemetryState *s, TraceEventType type,
                       uint32_t id, uint32_t value, char *device_name)
{
    if (g_async_queue_length(s->event_queue) >= TELEMETRY_QUEUE_MAX) {
        g_free(device_name);
        return; /* drop rather than block the TCG thread */
    }
    TraceEvent *ev = g_new(TraceEvent, 1);
    ev->timestamp_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    ev->type  = (uint8_t)type;
    ev->id    = id;
    ev->value = value;
    ev->device_name = device_name;
    g_async_queue_push(s->event_queue, ev);
}

static void telemetry_cpu_halt_hook(CPUState *cpu, bool halted)
{
    ZenohTelemetryState *s = global_telemetry;
    if (!s) return;
    int idx = cpu->cpu_index;
    if (idx < 0 || idx >= 32) return;
    if (halted == s->last_halted[idx]) return;
    s->last_halted[idx] = halted;
    send_event(s, TRACE_EVENT_CPU_STATE, (uint32_t)idx, halted ? 1 : 0, NULL);
}

static void telemetry_irq_hook(void *opaque, int n, int level)
{
    ZenohTelemetryState *s = global_telemetry;
    if (!s) return;
    uint32_t id = ((uint32_t)irq_slot_for(opaque) << 16) | (uint32_t)(n & 0xFFFF);
    
    /* 
     * CRITICAL FIX: opaque is a raw void*. Blindly casting to OBJECT(opaque) 
     * and reading obj->class causes segfaults if the opaque pointer isn't an Object.
     * Furthermore, resolving canonical paths outside the Big QEMU Lock (BQL) 
     * causes race conditions that corrupt QEMU memory.
     * We defer safe QOM resolution for a future architecture revision.
     */
    send_event(s, TRACE_EVENT_IRQ, id, (uint32_t)level, NULL);
}

static void zenoh_telemetry_realize(DeviceState *dev, Error **errp)
{
    ZenohTelemetryState *s = ZENOH_TELEMETRY(dev);
    if (global_telemetry) {
        error_setg(errp, "Only one zenoh-telemetry device allowed");
        return;
    }

    z_owned_config_t config;
    z_config_default(&config);
    if (s->router) {
        char json[256];
        snprintf(json, sizeof(json), "[\"%s\"]", s->router);
        zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", json);
        zc_config_insert_json5(z_config_loan_mut(&config), "scouting/multicast/enabled", "false");
    }

    if (z_open(&s->session, z_move(config), NULL) != 0) {
        error_setg(errp, "[zenoh-telemetry] failed to open Zenoh session");
        return;
    }
    s->session_open = true;

    char topic[128];
    snprintf(topic, sizeof(topic), "sim/telemetry/trace/%u", s->node_id);
    z_owned_keyexpr_t keyexpr;
    z_keyexpr_from_str(&keyexpr, topic);
    if (z_declare_publisher(z_session_loan(&s->session), &s->publisher,
                            z_keyexpr_loan(&keyexpr), NULL) != 0) {
        z_keyexpr_drop(z_move(keyexpr));
        z_session_drop(z_move(s->session));
        s->session_open = false;
        error_setg(errp, "[zenoh-telemetry] failed to declare publisher");
        return;
    }
    z_keyexpr_drop(z_move(keyexpr));

    s->event_queue   = g_async_queue_new();
    s->publish_thread = g_thread_new("telemetry-pub", telemetry_publish_thread, s);

    global_telemetry       = s;
    virtmcu_cpu_halt_hook  = telemetry_cpu_halt_hook;
    virtmcu_irq_hook       = telemetry_irq_hook;
}

static void zenoh_telemetry_instance_finalize(Object *obj)
{
    ZenohTelemetryState *s = ZENOH_TELEMETRY(obj);

    /* Stop hooks first so no new events are enqueued. */
    if (global_telemetry == s) {
        global_telemetry      = NULL;
        virtmcu_cpu_halt_hook = NULL;
        virtmcu_irq_hook      = NULL;
    }

    /* Signal publish thread to drain and exit, then wait for it. */
    if (s->publish_thread) {
        g_async_queue_push(s->event_queue, NULL); /* sentinel */
        g_thread_join(s->publish_thread);
        s->publish_thread = NULL;
    }

    if (s->event_queue) {
        g_async_queue_unref(s->event_queue);
        s->event_queue = NULL;
    }

    if (s->session_open) {
        z_undeclare_publisher(z_move(s->publisher));
        z_session_drop(z_move(s->session));
        s->session_open = false;
    }
}

static const Property zenoh_telemetry_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohTelemetryState, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohTelemetryState, router),
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
        .instance_size     = sizeof(ZenohTelemetryState),
        .instance_finalize = zenoh_telemetry_instance_finalize,
        .class_init        = zenoh_telemetry_class_init,
    },
};

DEFINE_TYPES(zenoh_telemetry_types)
module_obj(TYPE_ZENOH_TELEMETRY);

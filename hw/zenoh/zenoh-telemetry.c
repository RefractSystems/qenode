/*
 * hw/zenoh/zenoh-telemetry.c — Deterministic telemetry tracing.
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
#include <zenoh.h>

#define TYPE_ZENOH_TELEMETRY "zenoh-telemetry"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohTelemetryState, ZENOH_TELEMETRY)

typedef enum {
    TRACE_EVENT_CPU_STATE = 0,
    TRACE_EVENT_IRQ       = 1,
    TRACE_EVENT_PERIPHERAL = 2,
} TraceEventType;

typedef struct __attribute__((packed)) {
    uint64_t timestamp_ns;
    uint8_t  type;
    uint32_t id;
    uint32_t value;
} TraceEvent;

struct ZenohTelemetryState {
    SysBusDevice parent_obj;

    /* Properties */
    uint32_t node_id;
    char    *router;

    /* Zenoh handles */
    z_owned_session_t session;
    z_owned_keyexpr_t keyexpr;

    /* Internal state */
    bool last_halted;
};

static ZenohTelemetryState *global_telemetry;

static void send_event(ZenohTelemetryState *s, TraceEventType type, uint32_t id, uint32_t value)
{
    TraceEvent ev = {
        .timestamp_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL),
        .type = (uint8_t)type,
        .id = id,
        .value = value,
    };

    z_owned_bytes_t bytes;
    z_bytes_copy_from_buf(&bytes, (const uint8_t *)&ev, sizeof(ev));
    z_put(z_session_loan(&s->session), z_keyexpr_loan(&s->keyexpr), z_move(bytes), NULL);
}

static void telemetry_cpu_halt_hook(CPUState *cpu, bool halted)
{
    ZenohTelemetryState *s = global_telemetry;
    if (!s) return;
    send_event(s, TRACE_EVENT_CPU_STATE, 0, halted ? 1 : 0);
}

static void telemetry_irq_hook(void *opaque, int n, int level)
{
    ZenohTelemetryState *s = global_telemetry;
    if (!s) return;
    send_event(s, TRACE_EVENT_IRQ, (uint32_t)n, (uint32_t)level);
}

static void zenoh_telemetry_realize(DeviceState *dev, Error **errp)
{
    ZenohTelemetryState *s = ZENOH_TELEMETRY(dev);
    if (global_telemetry) {
        error_setg(errp, "Only one zenoh-telemetry device allowed");
        return;
    }
    global_telemetry = s;

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

    char topic[128];
    snprintf(topic, sizeof(topic), "sim/telemetry/trace/%u", s->node_id);
    z_keyexpr_from_str(&s->keyexpr, topic);

    virtmcu_cpu_halt_hook = telemetry_cpu_halt_hook;
    virtmcu_irq_hook = telemetry_irq_hook;
}

static void zenoh_telemetry_instance_finalize(Object *obj)
{
    ZenohTelemetryState *s = ZENOH_TELEMETRY(obj);
    if (global_telemetry == s) {
        global_telemetry = NULL;
        virtmcu_cpu_halt_hook = NULL;
        virtmcu_irq_hook = NULL;
    }
    z_keyexpr_drop(z_move(s->keyexpr));
    z_session_drop(z_move(s->session));
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

/*
 * hw/zenoh/zenoh-clock.c — External virtual clock synchronization.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "hw/core/sysbus.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"
#include "qapi/error.h"
#include "qemu/timer.h"
#include "qemu/main-loop.h"
#include "qemu/seqlock.h"
#include "system/cpus.h"
#include "system/cpu-timers.h"
#include "system/cpu-timers-internal.h"
#include "exec/icount.h"
#include "virtmcu/hooks.h"
#include <zenoh.h>

#define TYPE_ZENOH_CLOCK "zenoh-clock"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohClockState, ZENOH_CLOCK)

struct ZenohClockState {
    SysBusDevice parent_obj;

    /* Properties */
    uint32_t node_id;
    char *router;
    char *mode;

    /* State */
    z_owned_session_t session;
    z_owned_queryable_t queryable;
    QEMUTimer *quantum_timer;
    
    QemuMutex mutex;
    QemuCond vcpu_cond;
    QemuCond query_cond;
    
    bool is_icount;
    bool needs_quantum;
    bool quantum_granted;
    bool quantum_done;
};

static ZenohClockState *global_zenoh_clock = NULL;

typedef struct __attribute__((packed)) {
    uint64_t delta_ns;
    uint64_t mujoco_time_ns;
} ClockAdvancePayload;

typedef struct __attribute__((packed)) {
    uint64_t current_vtime_ns;
    uint32_t n_frames;
} ClockReadyPayload;

static void zclock_quantum_hook(CPUState *cpu)
{
    ZenohClockState *s = global_zenoh_clock;
    if (!s || s->is_icount) {
        return;
    }

    qemu_mutex_lock(&s->mutex);
    if (s->needs_quantum) {
        s->needs_quantum = false;
        s->quantum_done = true;
        qemu_cond_signal(&s->query_cond);
        
        s->quantum_granted = false;
        
        bool locked = bql_locked();
        if (locked) {
            bql_unlock();
        }
        while (!s->quantum_granted) {
            qemu_cond_wait(&s->vcpu_cond, &s->mutex);
        }
        if (locked) {
            bql_lock();
        }
    }
    qemu_mutex_unlock(&s->mutex);
}

static void zclock_timer_cb(void *opaque)
{
    ZenohClockState *s = opaque;
    qemu_mutex_lock(&s->mutex);
    s->needs_quantum = true;
    qemu_mutex_unlock(&s->mutex);

    CPUState *cpu;
    CPU_FOREACH(cpu) {
        cpu_exit(cpu);
    }
}

static void on_query(z_loaned_query_t *query, void *context)
{
    ZenohClockState *s = context;
    
    const z_loaned_bytes_t *payload_bytes = z_query_payload(query);
    if (!payload_bytes) {
        return;
    }

    ClockAdvancePayload req = {0};
    z_bytes_reader_t reader = z_bytes_get_reader(payload_bytes);
    z_bytes_reader_read(&reader, (uint8_t*)&req, sizeof(req));

    int64_t delta_ns = (int64_t)req.delta_ns;
    int64_t vtime = 0;

    if (s->is_icount) {
        bql_lock();
        int64_t current = qatomic_read(&timers_state.qemu_icount_bias);
        qatomic_set(&timers_state.qemu_icount_bias, current + delta_ns);
        qemu_clock_run_all_timers();
        vtime = icount_get();
        bql_unlock();
    } else {
        qemu_mutex_lock(&s->mutex);
        bql_lock();
        int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        int64_t target = now + delta_ns;
        s->quantum_done = false;
        timer_mod(s->quantum_timer, target);
        bql_unlock();
        
        s->quantum_granted = true;
        qemu_cond_signal(&s->vcpu_cond);
        
        while (!s->quantum_done) {
            qemu_cond_wait(&s->query_cond, &s->mutex);
        }
        
        bql_lock();
        vtime = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
        bql_unlock();
        qemu_mutex_unlock(&s->mutex);
    }

    ClockReadyPayload rep = {
        .current_vtime_ns = (uint64_t)vtime,
        .n_frames = 0,
    };
    
    z_owned_bytes_t reply_bytes;
    z_bytes_copy_from_buf(&reply_bytes, (const uint8_t*)&rep, sizeof(rep));
    z_query_reply(query, z_query_keyexpr(query), z_move(reply_bytes), NULL);
}

static void zenoh_clock_realize(DeviceState *dev, Error **errp)
{
    ZenohClockState *s = ZENOH_CLOCK(dev);
    
    if (global_zenoh_clock) {
        error_setg(errp, "Only one zenoh-clock device allowed");
        return;
    }
    
    global_zenoh_clock = s;
    
    qemu_mutex_init(&s->mutex);
    qemu_cond_init(&s->vcpu_cond);
    qemu_cond_init(&s->query_cond);

    if (s->mode && strcmp(s->mode, "icount") == 0) {
        s->is_icount = true;
    } else {
        s->is_icount = false;
        s->needs_quantum = true; /* Wait for first quantum immediately */
        s->quantum_granted = false;
        s->quantum_done = false;
        s->quantum_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, zclock_timer_cb, s);
        virtmcu_tcg_quantum_hook = zclock_quantum_hook;
    }

    z_owned_config_t config;
    z_config_default(&config);
    /* TODO: if router property is set, configure it. But for now, default is fine. */
    
    if (z_open(&s->session, z_move(config), NULL) != 0) {
        error_setg(errp, "Failed to open Zenoh session");
        return;
    }
    
    char topic[128];
    snprintf(topic, sizeof(topic), "sim/clock/advance/%u", s->node_id);
    
    z_owned_closure_query_t callback;
    z_closure_query(&callback, on_query, NULL, s);
    
    z_owned_keyexpr_t kexpr;
    if (z_keyexpr_from_str(&kexpr, topic) != 0) {
        error_setg(errp, "Failed to create Zenoh keyexpr: %s", topic);
        return;
    }
    
    if (z_declare_queryable(z_session_loan(&s->session), &s->queryable, z_keyexpr_loan(&kexpr), z_move(callback), NULL) != 0) {
        error_setg(errp, "Failed to declare Zenoh queryable on %s", topic);
        z_keyexpr_drop(z_move(kexpr));
        return;
    }
    z_keyexpr_drop(z_move(kexpr));
}

static const Property zenoh_clock_properties[] = {
    DEFINE_PROP_UINT32("node", ZenohClockState, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohClockState, router),
    DEFINE_PROP_STRING("mode", ZenohClockState, mode),
};

static void zenoh_clock_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_clock_realize;
    device_class_set_props(dc, zenoh_clock_properties);
    dc->user_creatable = true;
}

static const TypeInfo zenoh_clock_info = {
    .name          = TYPE_ZENOH_CLOCK,
    .parent        = TYPE_SYS_BUS_DEVICE,
    .instance_size = sizeof(ZenohClockState),
    .class_init    = zenoh_clock_class_init,
};

static void zenoh_clock_register_types(void)
{
    type_register_static(&zenoh_clock_info);
}

type_init(zenoh_clock_register_types)
module_obj(TYPE_ZENOH_CLOCK);

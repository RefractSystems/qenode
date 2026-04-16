/*
 * hw/zenoh/zenoh-clock.c — External virtual clock synchronization.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "qemu/seqlock.h"
#include "hw/core/sysbus.h"
#include "qom/object.h"
#include "hw/core/qdev-properties.h"
#include "qapi/error.h"
#include "qemu/timer.h"
#include "qemu/main-loop.h"
#include "system/cpus.h"
#include "system/cpu-timers.h"
#include "system/cpu-timers-internal.h"
#include "exec/icount.h"
#include "virtmcu/hooks.h"
#include "qemu/error-report.h"
#include <zenoh.h>

#define TYPE_ZENOH_CLOCK "zenoh-clock"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohClockState, ZENOH_CLOCK)

struct ZenohClockState {
    SysBusDevice parent_obj;

    /* Properties */
    uint32_t node_id;
    char    *router;
    char    *mode;

    /* Zenoh handles */
    z_owned_session_t    session;
    z_owned_queryable_t  queryable;

    /* Timer (suspend mode only) */
    QEMUTimer *quantum_timer;

    /*
     * Concurrency state — all fields below protected by mutex.
     */
    QemuMutex mutex;
    QemuCond  vcpu_cond;   /* on_query signals; vCPU hook waits here  */
    QemuCond  query_cond;  /* hook signals;     on_query waits here   */

    bool is_icount;

    /*
     * Suspend-mode handshake flags:
     */
    bool needs_quantum;
    bool quantum_ready;
    bool quantum_done;

    int64_t delta_ns;   /* on_query → hook: nanoseconds to advance         */
    int64_t vtime_ns;   /* hook → on_query: virtual clock after the quantum */
    
    int64_t mujoco_time_ns;         /* on_query → hook: current MuJoCo time */
    int64_t quantum_start_vtime_ns; /* hook → SAL/AAL: virtual clock at start. */
};

static ZenohClockState *global_zenoh_clock;

static void zclock_get_quantum_timing(VirtmcuQuantumTiming *timing)
{
    ZenohClockState *s = global_zenoh_clock;
    if (!s || !timing) return;
    timing->quantum_start_vtime_ns = qatomic_read(&s->quantum_start_vtime_ns);
    timing->quantum_delta_ns       = qatomic_read(&s->delta_ns);
    timing->mujoco_time_ns         = qatomic_read(&s->mujoco_time_ns);
}

typedef struct __attribute__((packed)) {
    uint64_t delta_ns;
    uint64_t mujoco_time_ns;
} ClockAdvancePayload;

typedef struct __attribute__((packed)) {
    uint64_t current_vtime_ns;
    uint32_t n_frames;
    uint32_t error_code; /* 0=OK, 1=STALL */
} ClockReadyPayload;

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

static void zclock_quantum_hook(CPUState *cpu)
{
    ZenohClockState *s = global_zenoh_clock;
    if (!s || !s->needs_quantum) return;

    bql_lock();
    qemu_mutex_lock(&s->mutex);
    if (!s->needs_quantum) {
        qemu_mutex_unlock(&s->mutex);
        bql_unlock();
        return;
    }

    s->needs_quantum = false;
    s->vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    s->quantum_done = true;
    qemu_cond_signal(&s->query_cond);
    bql_unlock();

    while (!s->quantum_ready) {
        qemu_cond_wait(&s->vcpu_cond, &s->mutex);
    }

    s->quantum_ready = false;
    s->quantum_done  = false;
    int64_t next_delta = qatomic_read(&s->delta_ns);
    qatomic_set(&s->quantum_start_vtime_ns, s->vtime_ns);
    qemu_mutex_unlock(&s->mutex);

    bql_lock();
    if (s->is_icount) {
        int64_t current = qatomic_read(&timers_state.qemu_icount_bias);
        qatomic_set(&timers_state.qemu_icount_bias, current + next_delta);
        qemu_clock_run_all_timers();
    }
    int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    timer_mod(s->quantum_timer, now + next_delta);
    bql_unlock();
}

static void on_query(z_loaned_query_t *query, void *context)
{
    ZenohClockState *s = context;
    const z_loaned_bytes_t *payload_bytes = z_query_payload(query);
    if (!payload_bytes) return;

    ClockAdvancePayload req = {0};
    z_bytes_reader_t reader = z_bytes_get_reader(payload_bytes);
    z_bytes_reader_read(&reader, (uint8_t *)&req, sizeof(req));

    qemu_mutex_lock(&s->mutex);
    qatomic_set(&s->delta_ns, (int64_t)req.delta_ns);
    qatomic_set(&s->mujoco_time_ns, (int64_t)req.mujoco_time_ns);
    
    s->quantum_done = false;
    s->quantum_ready = true;
    qemu_cond_signal(&s->vcpu_cond);

    uint32_t error_code = 0;
    /*
     * Wait for the hook with a 2-second timeout to detect QEMU stalls.
     * Must use a while loop — POSIX (and QEMU's wrapper) explicitly allows
     * spurious wakeups where timedwait returns 0 without quantum_done being
     * set.  A single if() would send a stale vtime back with error_code=0.
     */
    while (!s->quantum_done) {
        if (qemu_cond_timedwait(&s->query_cond, &s->mutex, 2000) != 0) {
            if (!s->quantum_done) {
                error_code = 1; /* STALL */
            }
            break;
        }
    }

    uint64_t vtime = (error_code == 0) ? (uint64_t)s->vtime_ns : 0;
    qemu_mutex_unlock(&s->mutex);

    ClockReadyPayload rep = {
        .current_vtime_ns = vtime,
        .n_frames         = 0,
        .error_code       = error_code,
    };

    z_owned_bytes_t reply_bytes;
    z_bytes_copy_from_buf(&reply_bytes, (const uint8_t *)&rep, sizeof(rep));
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

    s->is_icount = (s->mode && strcmp(s->mode, "icount") == 0);
    s->needs_quantum = true;
    s->quantum_ready = false;
    s->quantum_done  = false;
    s->quantum_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, zclock_timer_cb, s);
    virtmcu_tcg_quantum_hook = zclock_quantum_hook;
    virtmcu_get_quantum_timing = zclock_get_quantum_timing;

    z_owned_config_t config;
    z_config_default(&config);
    if (s->router) {
        char json[256];
        snprintf(json, sizeof(json), "[\"%s\"]", s->router);
        zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", json);
        zc_config_insert_json5(z_config_loan_mut(&config), "scouting/multicast/enabled", "false");
    }

    if (s->router) {
        fprintf(stderr, "[zenoh-clock] node=%u: connecting to router %s...\n",
                s->node_id, s->router);
    } else {
        fprintf(stderr,
                "[zenoh-clock] WARNING: node=%u: no router= set — falling back to "
                "multicast. This will NOT work in Docker/container environments "
                "(UDP multicast is dropped on macOS bridge networks). "
                "Set router=tcp/<host>:7447 for reliable operation.\n",
                s->node_id);
    }
    if (z_open(&s->session, z_move(config), NULL) != 0) {
        fprintf(stderr,
                "[zenoh-clock] node=%u: FATAL — failed to open Zenoh session "
                "(check router= value and ZENOH_CONFIG)\n",
                s->node_id);
        error_setg(errp, "Failed to open Zenoh session");
        return;
    }
    fprintf(stderr, "[zenoh-clock] node=%u: session opened successfully.\n", s->node_id);

    char topic[128];
    snprintf(topic, sizeof(topic), "sim/clock/advance/%u", s->node_id);
    z_owned_closure_query_t callback;
    z_closure_query(&callback, on_query, NULL, s);
    z_owned_keyexpr_t kexpr;
    z_keyexpr_from_str(&kexpr, topic);
    z_declare_queryable(z_session_loan(&s->session), &s->queryable, z_keyexpr_loan(&kexpr), z_move(callback), NULL);
    z_keyexpr_drop(z_move(kexpr));
}

static void zenoh_clock_instance_finalize(Object *obj)
{
    ZenohClockState *s = ZENOH_CLOCK(obj);
    if (global_zenoh_clock == s) global_zenoh_clock = NULL;
    virtmcu_tcg_quantum_hook = NULL;
    virtmcu_get_quantum_timing = NULL;
    if (s->quantum_timer) {
        timer_free(s->quantum_timer);
        s->quantum_timer = NULL;
    }
    z_queryable_drop(z_move(s->queryable));
    z_session_drop(z_move(s->session));
    qemu_cond_destroy(&s->query_cond);
    qemu_cond_destroy(&s->vcpu_cond);
    qemu_mutex_destroy(&s->mutex);
}

static const Property zenoh_clock_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohClockState, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohClockState, router),
    DEFINE_PROP_STRING("mode",   ZenohClockState, mode),
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
        .instance_size     = sizeof(ZenohClockState),
        .instance_finalize = zenoh_clock_instance_finalize,
        .class_init        = zenoh_clock_class_init,
    },
};

DEFINE_TYPES(zenoh_clock_types)
module_obj(TYPE_ZENOH_CLOCK);

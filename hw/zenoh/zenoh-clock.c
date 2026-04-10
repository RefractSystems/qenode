/*
 * hw/zenoh/zenoh-clock.c — External virtual clock synchronization.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 *
 * Implements two clock-slave modes selected by the "mode" device property:
 *
 * suspend (default):
 *   TCG runs at full speed between quanta.  At every TB boundary the
 *   zclock_quantum_hook checks whether the virtual timer has fired.  When it
 *   has, the hook blocks the vCPU thread and waits for the Zenoh
 *   TimeAuthority to supply the next delta_ns.
 *
 *   Lock ordering — must be followed strictly to prevent ABBA deadlock:
 *     vCPU thread:    BQL  →  s->mutex  (acquires BQL first, then mutex)
 *     on_query:              s->mutex   (acquires mutex only; NEVER calls
 *                                        bql_lock() in the suspend path)
 *
 *   State machine (suspend mode):
 *
 *     on_query stores delta_ns, sets quantum_ready, signals vcpu_cond.
 *     Hook (vCPU, BQL held) wakes: arms timer, returns (vCPU runs).
 *     Timer fires → timer_cb sets needs_quantum, kicks vCPU.
 *     Hook sees needs_quantum: captures vtime, sets quantum_done,
 *       signals query_cond, releases BQL, blocks on vcpu_cond.
 *     on_query wakes: reads vtime, sends reply, returns.
 *
 * icount:
 *   QEMU is started with -icount shift=0,align=off,sleep=off.
 *   on_query directly advances qemu_icount_bias under BQL and replies
 *   immediately.  The cooperative hook is disabled in this mode.
 *   BQL acquisition from the Zenoh thread is safe in icount mode because
 *   the hook is a no-op, eliminating the BQL→mutex dependency.
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
     *
     * Lock ordering:
     *   vCPU thread: acquire BQL, then mutex.
     *   on_query:    acquire mutex only.
     */
    QemuMutex mutex;
    QemuCond  vcpu_cond;   /* on_query signals; vCPU hook waits here  */
    QemuCond  query_cond;  /* hook signals;     on_query waits here   */

    bool is_icount;

    /*
     * Suspend-mode handshake flags:
     *
     *   needs_quantum  Set by timer_cb when the timer fires.  Cleared by
     *                  the hook when it begins handling the quantum boundary.
     *                  Signals the hook to block the vCPU.
     *
     *   quantum_ready  Set by on_query after it has written delta_ns.
     *                  Cleared by the hook after waking.
     *                  Signals the hook that a new delta is available.
     *
     *   quantum_done   Set by the hook after capturing vtime_ns.
     *                  Cleared by on_query before it starts waiting.
     *                  Signals on_query that vtime_ns is ready.
     */
    bool needs_quantum;
    bool quantum_ready;
    bool quantum_done;

    int64_t delta_ns;   /* on_query → hook: nanoseconds to advance         */
    int64_t vtime_ns;   /* hook → on_query: virtual clock after the quantum */
};

/* One global instance — enforced in realize(); cleared in finalize(). */
static ZenohClockState *global_zenoh_clock;

typedef struct __attribute__((packed)) {
    uint64_t delta_ns;
    uint64_t mujoco_time_ns;
} ClockAdvancePayload;

typedef struct __attribute__((packed)) {
    uint64_t current_vtime_ns;
    uint32_t n_frames;
} ClockReadyPayload;

/* ── Timer callback ──────────────────────────────────────────────────────────
 * Runs in the QEMU main-loop thread (BQL held).
 * Sets needs_quantum and kicks all vCPUs so the hook fires at the next TB.
 */
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

/* ── TCG quantum hook ────────────────────────────────────────────────────────
 * Installed as virtmcu_tcg_quantum_hook; called at every TB boundary from
 * the TCG thread.  BQL is held on entry and must be held on return.
 *
 * Fast path (needs_quantum == false): returns immediately.
 *
 * Slow path (needs_quantum == true):
 *  1. Clear needs_quantum so a concurrent timer_cb doesn't re-enter.
 *  2. Snapshot vtime_ns under BQL (while we still hold it).
 *  3. Set quantum_done and signal query_cond so on_query can wake.
 *  4. Release BQL — mandatory before any blocking wait.
 *  5. Wait on vcpu_cond until on_query sets quantum_ready.
 *  6. Clear quantum_ready, re-acquire BQL.
 *  7. Arm the timer for the next quantum.
 *  8. Return with BQL held (as required by the hook contract).
 */
static void zclock_quantum_hook(CPUState *cpu)
{
    ZenohClockState *s = global_zenoh_clock;
    if (!s) {
        return;
    }

    qemu_mutex_lock(&s->mutex);

    if (!s->needs_quantum) {
        /* Fast path: no quantum boundary pending. */
        qemu_mutex_unlock(&s->mutex);
        return;
    }

    /* Step 1: claim this quantum boundary. */
    s->needs_quantum = false;

    /* Step 2: acquire BQL to snapshot virtual time. */
    bql_lock();
    s->vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);

    /* Step 3: notify on_query that vtime_ns is ready. */
    s->quantum_done = true;
    qemu_cond_signal(&s->query_cond);

    /* Step 4: release BQL before blocking. */
    bql_unlock();

    /* Step 5: wait for on_query to deposit delta_ns. */
    while (!s->quantum_ready) {
        qemu_cond_wait(&s->vcpu_cond, &s->mutex);
    }

    /* Step 6: consume the ready flag, then re-acquire BQL. */
    s->quantum_ready = false;
    bql_lock();

    /* Step 7: arm the timer for the next quantum. */
    int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    timer_mod(s->quantum_timer, now + s->delta_ns);

    /* Step 8: release BQL and return. */
    bql_unlock();
    qemu_mutex_unlock(&s->mutex);
}

/* ── Zenoh queryable handler ─────────────────────────────────────────────────
 * Called from a Zenoh background thread.
 *
 * Suspend path: MUST NOT call bql_lock() — see lock-ordering comment above.
 * icount  path: bql_lock() is safe because the hook is disabled in this mode.
 */
static void on_query(z_loaned_query_t *query, void *context)
{
    ZenohClockState *s = context;

    const z_loaned_bytes_t *payload_bytes = z_query_payload(query);
    if (!payload_bytes) {
        return;
    }

    ClockAdvancePayload req = {0};
    z_bytes_reader_t reader = z_bytes_get_reader(payload_bytes);
    z_bytes_reader_read(&reader, (uint8_t *)&req, sizeof(req));

    int64_t vtime = 0;

    if (s->is_icount) {
        /*
         * icount mode: advance the icount bias directly.
         * No cooperative hook involvement — BQL acquisition is safe here.
         */
        bql_lock();
        int64_t current = qatomic_read(&timers_state.qemu_icount_bias);
        qatomic_set(&timers_state.qemu_icount_bias,
                    current + (int64_t)req.delta_ns);
        vtime = icount_get();
        bql_unlock();
    } else {
        /*
         * Suspend mode: coordinate with the vCPU hook.
         * NEVER call bql_lock() in this path.
         */
        qemu_mutex_lock(&s->mutex);

        /*
         * Deposit the next delta and wake the hook.
         * Reset quantum_done first so the subsequent wait is not
         * spuriously satisfied by a stale true from an earlier quantum.
         */
        s->delta_ns      = (int64_t)req.delta_ns;
        s->quantum_done  = false;
        s->quantum_ready = true;
        qemu_cond_signal(&s->vcpu_cond);

        /* Wait for the hook to capture vtime_ns after the quantum. */
        while (!s->quantum_done) {
            qemu_cond_wait(&s->query_cond, &s->mutex);
        }

        vtime = s->vtime_ns;
        qemu_mutex_unlock(&s->mutex);
    }

    ClockReadyPayload rep = {
        .current_vtime_ns = (uint64_t)vtime,
        .n_frames         = 0,
    };

    z_owned_bytes_t reply_bytes;
    z_bytes_copy_from_buf(&reply_bytes, (const uint8_t *)&rep, sizeof(rep));
    z_query_reply(query, z_query_keyexpr(query), z_move(reply_bytes), NULL);
}

/* ── Device lifecycle ────────────────────────────────────────────────────────*/

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
        s->is_icount     = false;
        s->needs_quantum = true;  /* Block vCPU immediately on first hook call. */
        s->quantum_ready = false;
        s->quantum_done  = false;
        s->quantum_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, zclock_timer_cb, s);
        virtmcu_tcg_quantum_hook = zclock_quantum_hook;
    }

    z_owned_config_t config;
    z_config_default(&config);

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

    if (z_declare_queryable(z_session_loan(&s->session), &s->queryable,
                            z_keyexpr_loan(&kexpr), z_move(callback), NULL) != 0) {
        error_setg(errp, "Failed to declare Zenoh queryable on %s", topic);
        z_keyexpr_drop(z_move(kexpr));
        return;
    }
    z_keyexpr_drop(z_move(kexpr));
}

static void zenoh_clock_instance_finalize(Object *obj)
{
    ZenohClockState *s = ZENOH_CLOCK(obj);

    if (global_zenoh_clock == s) {
        global_zenoh_clock = NULL;
    }

    if (!s->is_icount) {
        virtmcu_tcg_quantum_hook = NULL;
        if (s->quantum_timer) {
            timer_free(s->quantum_timer);
            s->quantum_timer = NULL;
        }
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

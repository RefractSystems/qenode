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
 *     vCPU hook:  BQL  →  s->mutex  (always acquires BQL first, then mutex)
 *     timer_cb:   BQL  →  s->mutex  (same order — no circular dependency)
 *     on_query:          s->mutex   (NEVER calls bql_lock() in suspend path)
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
 *   on_query performs the exact same handshake as suspend mode to ensure
 *   strict causal consistency. The qemu_icount_bias is advanced in Step 8
 *   of the hook, not directly in on_query.
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
    
    int64_t mujoco_time_ns;         /* on_query → hook: current MuJoCo time */
    int64_t quantum_start_vtime_ns; /* hook → SAL/AAL: virtual clock at start. Pair with delta_ns to interpolate: interpolated_ns = quantum_start_vtime_ns + fraction * delta_ns. */
};

/* One global instance — enforced in realize(); cleared in finalize(). */
static ZenohClockState *global_zenoh_clock;

static void zclock_get_quantum_timing(VirtmcuQuantumTiming *timing)
{
    ZenohClockState *s = global_zenoh_clock;
    if (!s || !timing) return;

    /*
     * Read concurrently. SAL/AAL models (running in vCPU thread) can safely
     * read this since the hook writes these variables before resuming vCPUs.
     */
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
 * the TCG thread.  In MTTCG, BQL is NOT held on entry or expected on return.
 * The hook acquires and releases BQL itself as needed.
 *
 * Lock ordering (identical to timer_cb — no ABBA possible):
 *   BQL → s->mutex
 *
 * Fast path (needs_quantum == false): returns after a single atomic check.
 *
 * Slow path (needs_quantum == true):
 *  1. Acquire BQL → s->mutex in that order (same as timer_cb).
 *  2. Re-check needs_quantum under the lock.
 *  3. Claim the quantum: clear needs_quantum, snapshot vtime_ns.
 *  4. Set quantum_done, signal query_cond so on_query can wake.
 *  5. Release BQL before blocking (condition_wait must not hold BQL).
 *  6. Wait on vcpu_cond (holding only s->mutex).
 *  7. Consume quantum_ready, release s->mutex.
 *  8. Acquire BQL, arm timer, release BQL, return.
 */
static void zclock_quantum_hook(CPUState *cpu)
{
    ZenohClockState *s = global_zenoh_clock;
    if (!s) {
        return;
    }

    /* Fast path: racy read is benign — bool reads are atomic on all QEMU targets. */
    if (!s->needs_quantum) {
        return;
    }

    /* Step 1: acquire BQL then mutex — same order as timer_cb. */
    bql_lock();
    qemu_mutex_lock(&s->mutex);

    /* Step 2: re-check under the lock (lost race with timer_cb or another vCPU). */
    if (!s->needs_quantum) {
        qemu_mutex_unlock(&s->mutex);
        bql_unlock();
        return;
    }

    /* Step 3: claim quantum, snapshot vtime under BQL. */
    s->needs_quantum = false;
    s->vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);

    /* Step 4: notify on_query. */
    s->quantum_done = true;
    qemu_cond_signal(&s->query_cond);

    /* Step 5: release BQL before blocking — timer_cb needs it to run. */
    bql_unlock();

    /* Step 6: wait for on_query to deposit the next delta_ns. */
    while (!s->quantum_ready) {
        qemu_cond_wait(&s->vcpu_cond, &s->mutex);
    }

    /* Step 7: consume quantum_ready, release mutex. */
    s->quantum_ready = false;
    
    int64_t next_delta = qatomic_read(&s->delta_ns);
    qatomic_set(&s->quantum_start_vtime_ns, s->vtime_ns);
    
    qemu_mutex_unlock(&s->mutex);

    /* Step 8: arm the timer for the next quantum (requires BQL). */
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

    /*
     * Coordinate with the vCPU hook.
     * NEVER call bql_lock() in this path.
     *
     * DESIGN NOTE: PLAN.md originally proposed that icount mode should
     * reply immediately in on_query. However, this breaks deterministic
     * memory queries via QMP (as seen in determinism_test.sh) because
     * the TimeAuthority proceeds before QEMU has actually finished its
     * instructions. We therefore use the same synchronous hook handshake
     * for both modes to guarantee strict causal consistency.
     */
    qemu_mutex_lock(&s->mutex);

    /*
     * Deposit the next delta and wake the hook.
     * Reset quantum_done first so the subsequent wait is not
     * spuriously satisfied by a stale true from an earlier quantum.
     */
    qatomic_set(&s->delta_ns, (int64_t)req.delta_ns);
    qatomic_set(&s->mujoco_time_ns, (int64_t)req.mujoco_time_ns);
    
    s->quantum_done  = false;
    s->quantum_ready = true;
    qemu_cond_signal(&s->vcpu_cond);

    /* Wait for the hook to capture vtime_ns after the quantum. */
    while (!s->quantum_done) {
        qemu_cond_wait(&s->query_cond, &s->mutex);
    }

    vtime = s->vtime_ns;
    qemu_mutex_unlock(&s->mutex);

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
        s->is_icount = false;
    }
    
    s->needs_quantum = true;  /* Block vCPU immediately on first hook call. */
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
        if (zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", json) != 0) {
            error_setg(errp, "Failed to set Zenoh router endpoint: %s", s->router);
            z_config_drop(z_move(config));
            return;
        }
        /*
         * Disable multicast scouting when an explicit router is set.
         * Multi-container environments (Docker Compose on macOS, Kubernetes)
         * drop multicast UDP between containers; the test must fail if QEMU
         * ignores router= and falls back to multicast peer discovery.
         */
        zc_config_insert_json5(z_config_loan_mut(&config),
                               "scouting/multicast/enabled", "false");
    }

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

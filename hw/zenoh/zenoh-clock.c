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
#include "virtmcu/hooks.h"
#include "virtmcu-rust-ffi.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohClockState ZenohClockState;

extern ZenohClockState *zenoh_clock_init(uint32_t node_id, const char *router,
                                         uint32_t stall_timeout_ms,
                                         QemuMutex *mutex, QemuCond *vcpu_cond, QemuCond *query_cond);
extern void             zenoh_clock_free(ZenohClockState *state);
extern int64_t          zenoh_clock_quantum_wait(ZenohClockState *state, int64_t current_vtime_ns);

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

    /* Synchronization */
    QemuMutex mutex;
    QemuCond  vcpu_cond;
    QemuCond  query_cond;
    
    int64_t   next_quantum_ns;
};

static ZenohClock *global_clock;

static void zenoh_clock_cpu_halt_cb(CPUState *cpu, bool halted)
{
    ZenohClock *s = global_clock;
    if (!s || !s->rust_state) {
        return;
    }

    /* We only care about the transition to HALTED (WFI) or regular quantum boundaries.
     * For simplicity, we just sync on every halt hook call if we reached the quantum. */
    int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    
    if (now >= s->next_quantum_ns || halted) {
        /*
         * BQL sandwich: release before blocking on a Zenoh reply so the QEMU
         * main loop (QMP, GDB stub, I/O) can make progress while we wait.
         */
        
        /* Debug logging (optional, can be noisy) */
        fprintf(stderr, "[zenoh-clock] sync at %lld ns (halted=%d, next=%lld)\n",
                (long long)now, halted, (long long)s->next_quantum_ns);

        ZenohClockState *rust_state = s->rust_state;
        
        /* Use our own BQL wrappers to avoid symbol resolution issues with TLS in DSOs */
        virtmcu_bql_unlock();

        int64_t delta = zenoh_clock_quantum_wait(rust_state, now);

        virtmcu_bql_lock();

        assert(s->rust_state != NULL &&
               "zenoh-clock finalized while blocking in quantum_wait");
        
        /* Update next quantum. If we entered due to halt, we might still be at the 
         * same time, but we now have a new budget. */
        s->next_quantum_ns = now + delta;
    }
}

static void zenoh_clock_tcg_quantum_cb(CPUState *cpu)
{
    zenoh_clock_cpu_halt_cb(cpu, false);
}

static void zenoh_clock_realize(DeviceState *dev, Error **errp)
{
    ZenohClock *s = ZENOH_CLOCK(dev);

    if (global_clock) {
        error_setg(errp, "Only one zenoh-clock instance is supported");
        return;
    }

    qemu_mutex_init(&s->mutex);
    qemu_cond_init(&s->vcpu_cond);
    qemu_cond_init(&s->query_cond);
    s->next_quantum_ns = 0;

    uint32_t stall_ms = s->stall_timeout_ms;
    if (stall_ms == 0) {
        const char *env = getenv("VIRTMCU_STALL_TIMEOUT_MS");
        stall_ms = (env && *env) ? (uint32_t)strtoul(env, NULL, 10) : 5000;
        if (stall_ms == 0) {
            stall_ms = 5000;
        }
    }

    s->rust_state = zenoh_clock_init(s->node_id, s->router, stall_ms,
                                     &s->mutex, &s->vcpu_cond, &s->query_cond);
    if (!s->rust_state) {
        error_setg(errp, "zenoh-clock: failed to initialize Rust backend (check Zenoh router/connectivity)");
        return;
    }

    global_clock = s;

    /* Register the vCPU halt hook for synchronization */
    virtmcu_cpu_halt_hook = zenoh_clock_cpu_halt_cb;
    virtmcu_tcg_quantum_hook = zenoh_clock_tcg_quantum_cb;
}

static void zenoh_clock_instance_finalize(Object *obj)
{
    ZenohClock *s = ZENOH_CLOCK(obj);
    if (s == global_clock) {
        virtmcu_cpu_halt_hook = NULL;
        virtmcu_tcg_quantum_hook = NULL;
        global_clock = NULL;
    }
    if (s->rust_state) {
        zenoh_clock_free(s->rust_state);
        s->rust_state = NULL;
    }
    qemu_mutex_destroy(&s->mutex);
    qemu_cond_destroy(&s->vcpu_cond);
    qemu_cond_destroy(&s->query_cond);
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

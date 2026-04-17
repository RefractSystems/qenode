/*
 * hw/zenoh/zenoh-clock-rust.c — Rust-backed virtual clock synchronization.
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

extern ZenohClockState *zenoh_clock_init(uint32_t node_id, const char *router, const char *mode);
extern void             zenoh_clock_fini(ZenohClockState *state);

/* Helper for Rust to advance icount bias without exposing timers_state struct */
void virtmcu_icount_advance(int64_t delta);
void virtmcu_icount_advance(int64_t delta)
{
    qatomic_set(&timers_state.qemu_icount_bias,
                qatomic_read(&timers_state.qemu_icount_bias) + delta);
}

/* ── C Wrappers for QEMU Macros ───────────────────────────────────────────── */
/* QEMU defines locking primitives as macros (which inject __FILE__, __LINE__) 
 * so we export clean C functions for Rust FFI. */

void virtmcu_bql_lock(void);
void virtmcu_bql_lock(void) { bql_lock(); }

void virtmcu_bql_unlock(void);
void virtmcu_bql_unlock(void) { bql_unlock(); }

void virtmcu_mutex_lock(QemuMutex *mutex);
void virtmcu_mutex_lock(QemuMutex *mutex) { qemu_mutex_lock(mutex); }

void virtmcu_mutex_unlock(QemuMutex *mutex);
void virtmcu_mutex_unlock(QemuMutex *mutex) { qemu_mutex_unlock(mutex); }

void virtmcu_cond_wait(QemuCond *cond, QemuMutex *mutex);
void virtmcu_cond_wait(QemuCond *cond, QemuMutex *mutex) { qemu_cond_wait(cond, mutex); }

int virtmcu_cond_timedwait(QemuCond *cond, QemuMutex *mutex, uint32_t ms);
int virtmcu_cond_timedwait(QemuCond *cond, QemuMutex *mutex, uint32_t ms) {
    return qemu_cond_timedwait(cond, mutex, ms);
}

void virtmcu_cond_signal(QemuCond *cond);
void virtmcu_cond_signal(QemuCond *cond) { qemu_cond_signal(cond); }

void virtmcu_cond_broadcast(QemuCond *cond);
void virtmcu_cond_broadcast(QemuCond *cond) { qemu_cond_broadcast(cond); }

/* QEMU timer functions are often static inlines */
QEMUTimer *virtmcu_timer_new_ns(QEMUClockType type, QEMUTimerCB *cb, void *opaque);
QEMUTimer *virtmcu_timer_new_ns(QEMUClockType type, QEMUTimerCB *cb, void *opaque) {
    return timer_new_ns(type, cb, opaque);
}

void virtmcu_timer_mod(QEMUTimer *ts, int64_t expire_time);
void virtmcu_timer_mod(QEMUTimer *ts, int64_t expire_time) {
    timer_mod(ts, expire_time);
}

void virtmcu_timer_free(QEMUTimer *ts);
void virtmcu_timer_free(QEMUTimer *ts) {
    timer_free(ts);
}

void virtmcu_cpu_exit_all(void);
void virtmcu_cpu_exit_all(void) {
    CPUState *cpu;
    CPU_FOREACH(cpu) {
        cpu_exit(cpu);
    }
}

/* ── QOM type ─────────────────────────────────────────────────────────────── */

#define TYPE_ZENOH_CLOCK_RUST "zenoh-clock-rust"
OBJECT_DECLARE_SIMPLE_TYPE(ZenohClockRust, ZENOH_CLOCK_RUST)

struct ZenohClockRust {
    SysBusDevice parent_obj;

    /* Properties */
    uint32_t node_id;
    char    *router;
    char    *mode;

    /* Rust state */
    ZenohClockState *rust_state;
};

static void zenoh_clock_rust_realize(DeviceState *dev, Error **errp)
{
    ZenohClockRust *s = ZENOH_CLOCK_RUST(dev);

    s->rust_state = zenoh_clock_init(s->node_id, s->router, s->mode);
    if (!s->rust_state) {
        error_setg(errp, "Failed to initialize Rust ZenohClock");
        return;
    }
}

static void zenoh_clock_rust_instance_finalize(Object *obj)
{
    ZenohClockRust *s = ZENOH_CLOCK_RUST(obj);
    if (s->rust_state) {
        zenoh_clock_fini(s->rust_state);
        s->rust_state = NULL;
    }
}

static const Property zenoh_clock_rust_properties[] = {
    DEFINE_PROP_UINT32("node",   ZenohClockRust, node_id, 0),
    DEFINE_PROP_STRING("router", ZenohClockRust, router),
    DEFINE_PROP_STRING("mode",   ZenohClockRust, mode),
};

static void zenoh_clock_rust_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = zenoh_clock_rust_realize;
    device_class_set_props(dc, zenoh_clock_rust_properties);
    dc->user_creatable = true;
}

static const TypeInfo zenoh_clock_rust_types[] = {
    {
        .name              = TYPE_ZENOH_CLOCK_RUST,
        .parent            = TYPE_SYS_BUS_DEVICE,
        .instance_size     = sizeof(ZenohClockRust),
        .instance_finalize = zenoh_clock_rust_instance_finalize,
        .class_init        = zenoh_clock_rust_class_init,
    },
};

DEFINE_TYPES(zenoh_clock_rust_types)
module_obj(TYPE_ZENOH_CLOCK_RUST);

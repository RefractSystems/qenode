/*
 * hw/misc/virtmcu-rust-ffi.c — Clean C wrappers for QEMU macros used by Rust.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "virtmcu-rust-ffi.h"
#include "qemu/main-loop.h"
#include "qemu/seqlock.h"
#include "hw/core/cpu.h"
#include "system/cpu-timers.h"
#include "system/cpu-timers-internal.h"
#include "exec/icount.h"

/* ── icount ──────────────────────────────────────────────────────────────── */

void virtmcu_icount_advance(int64_t delta)
{
    qatomic_set(&timers_state.qemu_icount_bias,
                qatomic_read(&timers_state.qemu_icount_bias) + delta);
    virtmcu_cpu_exit_all();
}

/* ── BQL ─────────────────────────────────────────────────────────────────── */

void virtmcu_bql_lock(void) { bql_lock(); }
void virtmcu_bql_unlock(void) { bql_unlock(); }

/* ── Mutex ───────────────────────────────────────────────────────────────── */

void virtmcu_mutex_lock(QemuMutex *mutex) { qemu_mutex_lock(mutex); }
void virtmcu_mutex_unlock(QemuMutex *mutex) { qemu_mutex_unlock(mutex); }

QemuMutex *virtmcu_mutex_new(void) {
    QemuMutex *m = g_new0(QemuMutex, 1);
    qemu_mutex_init(m);
    return m;
}

void virtmcu_mutex_free(QemuMutex *mutex) {
    qemu_mutex_destroy(mutex);
    g_free(mutex);
}

/* ── Cond ────────────────────────────────────────────────────────────────── */

void virtmcu_cond_wait(QemuCond *cond, QemuMutex *mutex) { qemu_cond_wait(cond, mutex); }

int virtmcu_cond_timedwait(QemuCond *cond, QemuMutex *mutex, uint32_t ms) {
    return qemu_cond_timedwait(cond, mutex, ms);
}

void virtmcu_cond_signal(QemuCond *cond) { qemu_cond_signal(cond); }
void virtmcu_cond_broadcast(QemuCond *cond) { qemu_cond_broadcast(cond); }

QemuCond *virtmcu_cond_new(void) {
    QemuCond *c = g_new0(QemuCond, 1);
    qemu_cond_init(c);
    return c;
}

void virtmcu_cond_free(QemuCond *cond) {
    qemu_cond_destroy(cond);
    g_free(cond);
}

/* ── Timer ───────────────────────────────────────────────────────────────── */

QEMUTimer *virtmcu_timer_new_ns(QEMUClockType type, QEMUTimerCB *cb, void *opaque) {
    return timer_new_ns(type, cb, opaque);
}

void virtmcu_timer_mod(QEMUTimer *ts, int64_t expire_time) {
    timer_mod(ts, expire_time);
}

void virtmcu_timer_free(QEMUTimer *ts) {
    timer_free(ts);
}

/* ── CPU ─────────────────────────────────────────────────────────────────── */

void virtmcu_cpu_exit_all(void) {
    CPUState *cpu;
    CPU_FOREACH(cpu) {
        cpu_exit(cpu);
    }
}

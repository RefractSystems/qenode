/*
 * hw/misc/virtmcu-rust-ffi.c — Clean C wrappers for QEMU macros used by Rust.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "ffi.h"
#include "qemu/main-loop.h"
#include "qemu/seqlock.h"
#include "hw/core/cpu.h"
#include "qapi/error.h"
#include "system/cpu-timers.h"
#include "system/cpu-timers-internal.h"
#include "system/runstate.h"
#include "exec/icount.h"
#include "hw/core/sysbus.h"
#include "chardev/char.h"
#include "chardev/char-fe.h"
#include "hw/ssi/ssi.h"

/* ── icount ──────────────────────────────────────────────────────────────── */

bool virtmcu_icount_enabled(void)
{
    return icount_enabled();
}

void virtmcu_icount_advance(int64_t delta)
{
    if (icount_enabled()) {
        qatomic_set(&timers_state.qemu_icount_bias,
                    qatomic_read(&timers_state.qemu_icount_bias) + delta);
    } else {
        timers_state.cpu_clock_offset += delta;
    }
}

/* ── BQL ─────────────────────────────────────────────────────────────────── */

/* Use the reliable helpers patched into QEMU's system/cpus.c to avoid 
 * TLS/symbol resolution issues when called from a DSO. */

bool virtmcu_bql_locked(void) {
#ifndef UNIT_TEST
    return virtmcu_is_bql_locked();
#else
    return false;
#endif
}

void virtmcu_bql_lock(void) {
#ifndef UNIT_TEST
    virtmcu_safe_bql_lock();
#endif
}

void virtmcu_bql_unlock(void) {
#ifndef UNIT_TEST
    virtmcu_safe_bql_unlock();
#endif
}

void virtmcu_bql_force_unlock(void) {
#ifndef UNIT_TEST
    virtmcu_safe_bql_unlock();
#endif
}

void virtmcu_bql_force_lock(void) {
#ifndef UNIT_TEST
    virtmcu_safe_bql_force_lock();
#endif
}

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

void virtmcu_cond_wait(QemuCond *cond, QemuMutex *mutex) {
    /* 300 s is long enough for any realistic quantum; a timeout here
     * means the sender's signal was lost — log and let the caller's
     * while-loop recheck the predicate rather than hanging forever. */
    if (!qemu_cond_timedwait(cond, mutex, 300000)) {
        fprintf(stderr, "[virtmcu] cond_wait: 300 s timeout — possible lost signal\n");
    }
}

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

void virtmcu_timer_del(QEMUTimer *ts) {
    timer_del(ts);
}

void virtmcu_timer_free(QEMUTimer *ts) {
    timer_free(ts);
}

/* ── CPU ─────────────────────────────────────────────────────────────────── */

/* These pointers are injected into QEMU via apply_zenoh_hook.py */
extern void (*virtmcu_tcg_quantum_hook)(CPUState *cpu);
extern void (*virtmcu_cpu_halt_hook)(CPUState *cpu, bool halted);

void virtmcu_cpu_exit_all(void) {
    CPUState *cpu;
    CPU_FOREACH(cpu) {
        cpu_exit(cpu);
    }
}

/* Setters are now implemented in QEMU (cpu-exec.c) */

/* ── Error ───────────────────────────────────────────────────────────────── */

bool virtmcu_runstate_is_running(void)
{
    return runstate_is_running();
}

void virtmcu_error_setg(Error **errp, const char *fmt)
{
    error_setg_internal(errp, "rust", 0, "rust", "%s", fmt);
}

void virtmcu_log(const char *fmt)
{
    fprintf(stderr, "%s", fmt);
    fflush(stderr);
}

/* ── Sizes ───────────────────────────────────────────────────────────────── */

size_t virtmcu_sizeof_device_state(void) { return sizeof(struct DeviceState); }
size_t virtmcu_sizeof_sys_bus_device(void) { return sizeof(struct SysBusDevice); }
size_t virtmcu_sizeof_device_class(void) { return sizeof(struct DeviceClass); }
size_t virtmcu_sizeof_ssi_peripheral(void) { return sizeof(struct SSIPeripheral); }
size_t virtmcu_sizeof_ssi_peripheral_class(void) { return sizeof(struct SSIPeripheralClass); }
size_t virtmcu_sizeof_chardev(void) { return sizeof(struct Chardev); }
size_t virtmcu_sizeof_chardev_class(void) { return sizeof(struct ChardevClass); }
size_t virtmcu_sizeof_char_backend(void) { return sizeof(struct CharFrontend); }

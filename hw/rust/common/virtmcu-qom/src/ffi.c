/*
 * hw/misc/virtmcu-rust-ffi.c — Clean C wrappers for QEMU macros used by Rust.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#ifdef UNIT_TEST
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Weak stubs to allow linking and avoid crashes, while letting tests override them */
__attribute__((weak)) void register_dso_module_init(void (*fn)(void), int type) { (void)fn; (void)type; }
__attribute__((weak)) void *type_register_static(const void *info) { (void)info; return NULL; }
__attribute__((weak)) int qemu_loglevel = 0;
__attribute__((weak)) void virtmcu_log(const char *fmt) { fprintf(stderr, "%s", fmt); }
__attribute__((weak)) void virtmcu_error_setg(void **errp, const char *fmt, ...) { (void)errp; (void)fmt; }
__attribute__((weak)) bool virtmcu_runstate_is_running(void) { return true; }
__attribute__((weak)) void virtmcu_cpu_exit_all(void) {}

/* QOM/Device stubs */
__attribute__((weak)) void *object_class_dynamic_cast_assert(void *klass, const char *typename, const char *file, int line, const char *func) { (void)typename; (void)file; (void)line; (void)func; return klass; }
__attribute__((weak)) void *object_dynamic_cast(void *obj, const char *typename) { (void)obj; (void)typename; return obj; }
__attribute__((weak)) void *object_property_get_uint(void *obj, const char *name, void **errp) { (void)obj; (void)name; (void)errp; return NULL; }
__attribute__((weak)) void object_property_set_bool(void *obj, const char *name, bool value, void **errp) { (void)obj; (void)name; (void)value; (void)errp; }
__attribute__((weak)) void object_class_property_add_link(void *oc, const char *name, const char *type, int offset, void *check, int flags) { (void)oc; (void)name; (void)type; (void)offset; (void)check; (void)flags; }
__attribute__((weak)) void object_class_property_add_str(void *oc, const char *name, void *get, void *set) { (void)oc; (void)name; (void)get; (void)set; }
__attribute__((weak)) void object_property_add_uint64_ptr(void *obj, const char *name, const uint64_t *v, int flags) { (void)obj; (void)name; (void)v; (void)flags; }
__attribute__((weak)) void *object_get_root(void) { return NULL; }
__attribute__((weak)) void object_child_foreach_recursive(void *obj, void *fn, void *opaque) { (void)obj; (void)fn; (void)opaque; }
__attribute__((weak)) char *object_get_canonical_path(void *obj) { (void)obj; return NULL; }
__attribute__((weak)) void device_class_set_props_n(void *dc, void *props, int n) { (void)dc; (void)props; (void)n; }
__attribute__((weak)) void memory_region_init_io(void *mr, void *owner, const void *ops, void *opaque, const char *name, uint64_t size) { (void)mr; (void)owner; (void)ops; (void)opaque; (void)name; (void)size; }
__attribute__((weak)) void sysbus_init_mmio(void *dev, void *mr) { (void)dev; (void)mr; }
__attribute__((weak)) void sysbus_mmio_map(void *dev, int n, uint64_t addr) { (void)dev; (void)n; (void)addr; }
__attribute__((weak)) void sysbus_init_irq(void *dev, void **irqp) { (void)dev; (void)irqp; }
__attribute__((weak)) void qemu_set_irq(void *irq, int level) { (void)irq; (void)level; }
__attribute__((weak)) void *sysbus_get_connected_irq(void *dev, int n) { (void)dev; (void)n; return NULL; }

/* Chardev stubs */
__attribute__((weak)) void qemu_chr_fe_set_handlers(void *be, void *can_receive, void *receive, void *event, void *be_change, void *opaque, void *context, bool set_built_in) { (void)be; (void)can_receive; (void)receive; (void)event; (void)be_change; (void)opaque; (void)context; (void)set_built_in; }
__attribute__((weak)) int qemu_chr_fe_write(void *be, const uint8_t *buf, int len) { (void)be; (void)buf; return len; }
__attribute__((weak)) int qemu_chr_be_can_write(void *s) { (void)s; return 1024; }
__attribute__((weak)) void qemu_chr_be_write(void *s, uint8_t *buf, int len) { (void)s; (void)buf; (void)len; }
__attribute__((weak)) void qemu_chr_parse_common(void *opts, void *backend) { (void)opts; (void)backend; }

/* Netdev stubs */
__attribute__((weak)) void *qemu_new_net_client(void *info, void *peer, const char *model, const char *name) { (void)info; (void)peer; (void)model; (void)name; return NULL; }
__attribute__((weak)) bool qemu_can_receive_packet(void *nc) { (void)nc; return true; }
__attribute__((weak)) void qemu_send_packet(void *nc, const uint8_t *buf, int size) { (void)nc; (void)buf; (void)size; }
__attribute__((weak)) void *virtmcu_netdev_hook = NULL;

/* Property stubs */
__attribute__((weak)) void *qdev_prop_string = NULL;
__attribute__((weak)) void *qdev_prop_uint32 = NULL;
__attribute__((weak)) void *qdev_prop_uint64 = NULL;
__attribute__((weak)) void *qdev_prop_bool = NULL;
__attribute__((weak)) void *qdev_prop_macaddr = NULL;
__attribute__((weak)) void *qdev_prop_chr = NULL;

/* Clock/Timer stubs */
__attribute__((weak)) int64_t qemu_clock_get_ns(int type) { (void)type; return 0; }
__attribute__((weak)) void *timer_new_ns(int type, void *cb, void *opaque) { (void)type; (void)cb; (void)opaque; return NULL; }
__attribute__((weak)) void timer_mod(void *ts, int64_t expire_time) { (void)ts; (void)expire_time; }
__attribute__((weak)) void timer_del(void *ts) { (void)ts; }
__attribute__((weak)) void timer_free(void *ts) { (void)ts; }
__attribute__((weak)) void virtmcu_timer_kick(void) {}

__attribute__((weak)) void *virtmcu_timer_new_ns(int type, void *cb, void *opaque) { return NULL; }
__attribute__((weak)) void virtmcu_timer_mod(void *ts, int64_t expire_time) {}
__attribute__((weak)) void virtmcu_timer_del(void *ts) {}
__attribute__((weak)) void virtmcu_timer_free(void *ts) {}

/* icount stubs */
__attribute__((weak)) bool virtmcu_icount_enabled(void) { return false; }
__attribute__((weak)) void virtmcu_icount_advance(int64_t delta) {}

/* Sync stubs */
__attribute__((weak)) void qemu_mutex_init(void *m) { (void)m; }
__attribute__((weak)) void qemu_mutex_destroy(void *m) { (void)m; }
__attribute__((weak)) void qemu_mutex_lock(void *m) { (void)m; }
__attribute__((weak)) void qemu_mutex_unlock(void *m) { (void)m; }
__attribute__((weak)) void qemu_cond_init(void *c) { (void)c; }
__attribute__((weak)) void qemu_cond_destroy(void *c) { (void)c; }
__attribute__((weak)) void qemu_cond_wait(void *c, void *m) { (void)c; (void)m; }
__attribute__((weak)) void qemu_cond_signal(void *c) { (void)c; }
__attribute__((weak)) void qemu_cond_broadcast(void *c) { (void)c; }
__attribute__((weak)) bool qemu_cond_timedwait(void *c, void *m, int ms) { (void)c; (void)m; (void)ms; return true; }

__attribute__((weak)) void *virtmcu_mutex_new(void) { return NULL; }
__attribute__((weak)) void virtmcu_mutex_free(void *m) { (void)m; }
__attribute__((weak)) void virtmcu_mutex_lock(void *m) { (void)m; }
__attribute__((weak)) void virtmcu_mutex_unlock(void *m) { (void)m; }
__attribute__((weak)) void *virtmcu_cond_new(void) { return NULL; }
__attribute__((weak)) void virtmcu_cond_free(void *c) { (void)c; }
__attribute__((weak)) void virtmcu_cond_broadcast(void *c) { (void)c; }
__attribute__((weak)) bool virtmcu_cond_timedwait(void *c, void *m, uint32_t ms) { (void)c; (void)m; (void)ms; return true; }

__attribute__((weak)) bool virtmcu_bql_locked(void) { return true; }
__attribute__((weak)) void virtmcu_bql_lock(void) {}
__attribute__((weak)) void virtmcu_bql_unlock(void) {}
__attribute__((weak)) void virtmcu_bql_force_lock(void) {}
__attribute__((weak)) void virtmcu_bql_force_unlock(void) {}

__attribute__((weak)) bool virtmcu_is_bql_locked(void) { return true; }
__attribute__((weak)) void virtmcu_safe_bql_lock(void) {}
__attribute__((weak)) void virtmcu_safe_bql_unlock(void) {}
__attribute__((weak)) void virtmcu_safe_bql_force_lock(void) {}
__attribute__((weak)) void virtmcu_safe_bql_force_unlock(void) {}

/* CPU Hooks */
__attribute__((weak)) void virtmcu_cpu_set_halt_hook(void (*cb)(void *, bool)) { (void)cb; }
__attribute__((weak)) void virtmcu_cpu_set_tcg_hook(void (*cb)(void *)) { (void)cb; }
__attribute__((weak)) bool virtmcu_vcpu_should_yield(void *cpu) { return false; }
__attribute__((weak)) void virtmcu_kick_first_cpu_for_quantum(void) {}

/* GLib stubs */
__attribute__((weak)) void *g_malloc0(size_t n) { return calloc(1, n); }
__attribute__((weak)) void g_free(void *p) { free(p); }
__attribute__((weak)) char *g_strdup(const char *s) { return s ? strdup(s) : NULL; }
__attribute__((weak)) char *g_strdup_printf(const char *fmt, ...) { (void)fmt; return NULL; }

/* CAN stubs */
__attribute__((weak)) void can_bus_client_send(void *c, void *f, int n) { (void)c; (void)f; (void)n; }
__attribute__((weak)) void can_bus_insert_client(void *b, void *c) { (void)b; (void)c; }
__attribute__((weak)) void can_bus_remove_client(void *c) { (void)c; }

/* Opts stubs */
__attribute__((weak)) const char *qemu_opt_get(void *opts, const char *name) { (void)opts; (void)name; return NULL; }
__attribute__((weak)) uint64_t qemu_opt_get_size(void *opts, const char *name, uint64_t def) { (void)opts; (void)name; return def; }
__attribute__((weak)) uint64_t qemu_opt_get_number(void *opts, const char *name, uint64_t def) { (void)opts; (void)name; return def; }

/* Sizes */
__attribute__((weak)) size_t virtmcu_sizeof_device_state(void) { return 1024; }
__attribute__((weak)) size_t virtmcu_sizeof_sys_bus_device(void) { return 1024; }
__attribute__((weak)) size_t virtmcu_sizeof_device_class(void) { return 1024; }
__attribute__((weak)) size_t virtmcu_sizeof_ssi_peripheral(void) { return 1024; }
__attribute__((weak)) size_t virtmcu_sizeof_ssi_peripheral_class(void) { return 1024; }
__attribute__((weak)) size_t virtmcu_sizeof_chardev(void) { return 1024; }
__attribute__((weak)) size_t virtmcu_sizeof_chardev_class(void) { return 1024; }
__attribute__((weak)) size_t virtmcu_sizeof_char_backend(void) { return 1024; }

#else

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

bool virtmcu_bql_locked(void) {
    return virtmcu_is_bql_locked();
}

void virtmcu_bql_lock(void) {
    // Task 12: Safe BQL yield pattern.
    // In peripheral threads, we use the safe wrapper that handles
    // recursive locking and ensures we are not already holding it.
    virtmcu_safe_bql_lock();
}

void virtmcu_bql_unlock(void) {
    virtmcu_safe_bql_unlock();
}

void virtmcu_bql_force_unlock(void) {
    virtmcu_safe_bql_unlock();
}

void virtmcu_bql_force_lock(void) {
    // For extreme cases where we must ensure BQL regardless of current state.
    // Use with caution.
    virtmcu_safe_bql_force_lock();
}

/* ── Mutex/Cond ──────────────────────────────────────────────────────────── */

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

void virtmcu_cond_wait(QemuCond *cond, QemuMutex *mutex) {
    qemu_cond_wait(cond, mutex);
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

/* ── Timers ──────────────────────────────────────────────────────────────── */

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

/* ── CPU / Core Hooks ────────────────────────────────────────────────────── */

extern void (*virtmcu_tcg_quantum_hook)(CPUState *cpu);
extern void (*virtmcu_cpu_halt_hook)(CPUState *cpu, bool halted);

void virtmcu_cpu_exit_all(void)
{
    CPUState *cpu;
    CPU_FOREACH(cpu) {
        cpu_exit(cpu);
    }
}

/* ── Runstate / Control ──────────────────────────────────────────────────── */

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

#endif

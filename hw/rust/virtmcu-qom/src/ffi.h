/*
 * hw/misc/virtmcu-rust-ffi.h — Clean C wrappers for QEMU macros used by Rust.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#ifndef VIRTMCU_RUST_FFI_H
#define VIRTMCU_RUST_FFI_H

/* Suppress noisy warnings from QEMU system headers */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wsign-compare"

#include "qemu/osdep.h"
#include "qemu/main-loop.h"
#include "hw/core/cpu.h"
#include "qapi/error.h"
#include "system/cpu-timers.h"

#pragma GCC diagnostic pop

/* icount */
bool virtmcu_icount_enabled(void);
void virtmcu_icount_advance(int64_t delta);

/* BQL */
bool virtmcu_bql_locked(void);
void virtmcu_bql_lock(void);
void virtmcu_bql_unlock(void);

/* Mutex */
void virtmcu_mutex_lock(QemuMutex *mutex);
void virtmcu_mutex_unlock(QemuMutex *mutex);
QemuMutex *virtmcu_mutex_new(void);
void virtmcu_mutex_free(QemuMutex *mutex);

/* Cond */
void virtmcu_cond_wait(QemuCond *cond, QemuMutex *mutex);
int virtmcu_cond_timedwait(QemuCond *cond, QemuMutex *mutex, uint32_t ms);
void virtmcu_cond_signal(QemuCond *cond);
void virtmcu_cond_broadcast(QemuCond *cond);
QemuCond *virtmcu_cond_new(void);
void virtmcu_cond_free(QemuCond *cond);

/* Timer */
QEMUTimer *virtmcu_timer_new_ns(QEMUClockType type, QEMUTimerCB *cb, void *opaque);
void virtmcu_timer_mod(QEMUTimer *ts, int64_t expire_time);
void virtmcu_timer_del(QEMUTimer *ts);
void virtmcu_timer_free(QEMUTimer *ts);

/* CPU */
void virtmcu_cpu_exit_all(void);
void virtmcu_cpu_set_halt_hook(void (*cb)(CPUState *, bool));
void virtmcu_cpu_set_tcg_hook(void (*cb)(CPUState *));

/* Error */
void virtmcu_error_setg(Error **errp, const char *fmt);
void virtmcu_log(const char *fmt);

/* Sizes */
size_t virtmcu_sizeof_device_state(void);
size_t virtmcu_sizeof_sys_bus_device(void);
size_t virtmcu_sizeof_device_class(void);
size_t virtmcu_sizeof_ssi_peripheral(void);
size_t virtmcu_sizeof_ssi_peripheral_class(void);
size_t virtmcu_sizeof_chardev(void);
size_t virtmcu_sizeof_chardev_class(void);
size_t virtmcu_sizeof_char_backend(void);

#endif

/*
 * virtmcu rust-dummy QOM device.
 *
 * Demonstrates QEMU C/Rust interoperability for dynamic module plugins.
 * QOM registration and MemoryRegion setup remain in C (QEMU's native language).
 * MMIO read/write callbacks are forwarded to an extern "C" Rust static library.
 *
 * Design pattern for extending this template:
 *   - Add Rust-owned state by storing it in RustDummyState.rust_priv (void *).
 *   - Export rust_dummy_init() / rust_dummy_fini() from Rust to allocate/free it.
 *   - Call rust_dummy_init() from rust_dummy_realize() and store the result.
 *   - Pass rust_priv as the first argument to rust_dummy_read/write so Rust can
 *     access its own state through a typed pointer.
 *
 * Why raw FFI instead of QEMU's native Rust build?
 *   QEMU 10+ has an official Meson+cargo integration, but it compiles Rust into
 *   the monolithic qemu-system-* binary — it does not produce loadable .so modules.
 *   Our approach (rustc → staticlib → linked into .so) works with --enable-modules
 *   and keeps peripheral code out of the QEMU source tree.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "qemu/osdep.h"
#include "qemu/log.h"
#include "qemu/module.h"
#include "hw/core/sysbus.h"
#include "hw/core/qdev-properties.h"
#include "qom/object.h"

/* ── Rust FFI declarations ──────────────────────────────────────────────────
 *
 * All Rust functions receive 'priv_state' (the device's opaque private pointer)
 * as their first argument.  This is NULL in the current stateless demo, but
 * real devices should allocate Rust state in rust_dummy_realize() and pass it
 * here so the Rust side can access per-instance data without global variables.
 *
 * Signatures must exactly match the #[no_mangle] pub extern "C" functions in
 * hw/rust-dummy/src/lib.rs.  The caller (C) is responsible for ABI correctness.
 */
extern uint64_t rust_dummy_read(void *priv_state, uint64_t addr, uint32_t size);
extern void     rust_dummy_write(void *priv_state, uint64_t addr, uint64_t val,
                                 uint32_t size);

/* ── QOM type ───────────────────────────────────────────────────────────────*/

#define TYPE_RUST_DUMMY "rust-dummy"
OBJECT_DECLARE_SIMPLE_TYPE(RustDummyState, RUST_DUMMY)

struct RustDummyState {
    SysBusDevice parent_obj;
    MemoryRegion iomem;

    /* Optional: pointer to Rust-owned state allocated by rust_dummy_init().
     * NULL in the current stateless demo — see design pattern note above. */
    void        *rust_priv;

    /* Properties */
    uint64_t     base_addr;
};

/* ── MemoryRegion callbacks ─────────────────────────────────────────────────*/

static uint64_t rust_dummy_mmio_read(void *opaque, hwaddr addr, unsigned size)
{
    RustDummyState *s = RUST_DUMMY(opaque);
    uint64_t val = rust_dummy_read(s->rust_priv, (uint64_t)addr, (uint32_t)size);

    qemu_log_mask(LOG_UNIMP,
                  "rust-dummy: read  addr=0x%03" HWADDR_PRIx
                  " size=%u val=0x%" PRIx64 "\n",
                  addr, size, val);
    return val;
}

static void rust_dummy_mmio_write(void *opaque, hwaddr addr, uint64_t val,
                                  unsigned size)
{
    RustDummyState *s = RUST_DUMMY(opaque);

    qemu_log_mask(LOG_UNIMP,
                  "rust-dummy: write addr=0x%03" HWADDR_PRIx
                  " size=%u val=0x%" PRIx64 "\n",
                  addr, size, val);
    rust_dummy_write(s->rust_priv, (uint64_t)addr, val, (uint32_t)size);
}

static const MemoryRegionOps rust_dummy_ops = {
    .read      = rust_dummy_mmio_read,
    .write     = rust_dummy_mmio_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .impl = {
        .min_access_size = 1,
        .max_access_size = 8,
    },
};

/* ── Device lifecycle ───────────────────────────────────────────────────────*/

static void rust_dummy_realize(DeviceState *dev, Error **errp)
{
    RustDummyState *s = RUST_DUMMY(dev);

    /*
     * Stateful extension point: if the Rust side needs per-instance data,
     * export rust_dummy_init() → *void from lib.rs, call it here, and store
     * the result in s->rust_priv.  Then pass it to every read/write call.
     *
     *   s->rust_priv = rust_dummy_init();
     *   if (!s->rust_priv) {
     *       error_setg(errp, "rust-dummy: Rust init failed");
     *       return;
     *   }
     */
    s->rust_priv = NULL;  /* stateless demo */

    memory_region_init_io(&s->iomem, OBJECT(s), &rust_dummy_ops, s,
                          TYPE_RUST_DUMMY, 0x1000);
    sysbus_init_mmio(SYS_BUS_DEVICE(s), &s->iomem);

    if (s->base_addr != UINT64_MAX) {
        sysbus_mmio_map(SYS_BUS_DEVICE(s), 0, s->base_addr);
    }
}

static const Property rust_dummy_props[] = {
    DEFINE_PROP_UINT64("base-addr", RustDummyState, base_addr, UINT64_MAX),
};

static void rust_dummy_class_init(ObjectClass *klass, const void *data)
{
    DeviceClass *dc = DEVICE_CLASS(klass);
    dc->realize = rust_dummy_realize;
    device_class_set_props(dc, rust_dummy_props);
    dc->user_creatable = true;
}

static const TypeInfo rust_dummy_types[] = {
    {
        .name          = TYPE_RUST_DUMMY,
        .parent        = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(RustDummyState),
        .class_init    = rust_dummy_class_init,
    },
};

DEFINE_TYPES(rust_dummy_types)
module_obj(TYPE_RUST_DUMMY);

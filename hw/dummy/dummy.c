/*
 * qenode dummy QOM device — minimal SysBusDevice proving dynamic .so loading.
 *
 * Use this as the canonical template when adding new peripheral models.
 * Anything not needed here (vmstate, reset, properties) should be added when
 * you actually need it — don't copy dead code into new devices.
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

/*
 * qemu/osdep.h MUST be the very first include. It pulls in config-host.h and
 * resolves platform differences before any other header sees them.
 */
#include "qemu/osdep.h"
#include "qemu/log.h"
#include "hw/core/sysbus.h"
#include "qom/object.h"

/* ── Type registration ────────────────────────────────────────────────────── */

#define TYPE_DUMMY_DEVICE "dummy-device"

/*
 * OBJECT_DECLARE_SIMPLE_TYPE(StateType, TYPE_MACRO) expands to:
 *   - a forward-declare of the struct
 *   - a DUMMY_DEVICE() cast macro
 * It must appear before the struct definition.
 */
OBJECT_DECLARE_SIMPLE_TYPE(DummyDeviceState, DUMMY_DEVICE)

/* ── Device state ─────────────────────────────────────────────────────────── */

struct DummyDeviceState {
    /* parent_obj must be first — the QOM cast macros rely on this layout. */
    SysBusDevice parent_obj;

    MemoryRegion mmio;
};

/* ── MMIO callbacks ───────────────────────────────────────────────────────── */

#define DUMMY_MMIO_SIZE 0x1000

static uint64_t dummy_read(void *opaque, hwaddr addr, unsigned size)
{
    qemu_log_mask(LOG_UNIMP,
                  "dummy-device: unimplemented read  addr=0x%"PRIx64
                  " size=%u\n", addr, size);
    return 0;
}

static void dummy_write(void *opaque, hwaddr addr, uint64_t val, unsigned size)
{
    qemu_log_mask(LOG_UNIMP,
                  "dummy-device: unimplemented write addr=0x%"PRIx64
                  " val=0x%"PRIx64 " size=%u\n", addr, val, size);
}

static const MemoryRegionOps dummy_mmio_ops = {
    .read  = dummy_read,
    .write = dummy_write,
    .impl  = {
        .min_access_size = 4,
        .max_access_size = 4,
    },
    .endianness = DEVICE_LITTLE_ENDIAN,
};

/* ── Initialization ───────────────────────────────────────────────────────── */

/*
 * instance_init: called by object_new() / object_initialize().
 * Allocate and initialise sub-objects here (MemoryRegions, QEMUTimers, …).
 * Do NOT rely on properties being set at this stage.
 */
static void dummy_init(Object *obj)
{
    DummyDeviceState *s = DUMMY_DEVICE(obj);
    SysBusDevice *sbd   = SYS_BUS_DEVICE(obj);

    memory_region_init_io(&s->mmio, obj, &dummy_mmio_ops, s,
                          TYPE_DUMMY_DEVICE, DUMMY_MMIO_SIZE);
    sysbus_init_mmio(sbd, &s->mmio);
}

/* ── TypeInfo / registration ──────────────────────────────────────────────── */

/*
 * Use a static array + DEFINE_TYPES() rather than type_register_static() +
 * type_init().  DEFINE_TYPES() is the modern idiom since QEMU 7.x — it wires
 * up a __attribute__((constructor)) for free, which is what fires when this
 * .so is loaded via LD_PRELOAD or dlopen().
 */
static const TypeInfo dummy_types[] = {
    {
        .name          = TYPE_DUMMY_DEVICE,
        .parent        = TYPE_SYS_BUS_DEVICE,
        .instance_size = sizeof(DummyDeviceState),
        .instance_init = dummy_init,
    },
};

DEFINE_TYPES(dummy_types)

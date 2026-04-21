/*
 * Minimal Educational C Dummy Device for virtmcu.
 * This device serves as an example of a legacy C QOM peripheral.
 *
 * NOTE: For new peripherals, refer to hw/rust/rust-dummy/ as the preferred
 * Rust-native approach.
 */

// clang-format off
#include "qemu/osdep.h"
// clang-format on
#include "hw/core/sysbus.h"
#include "qemu/log.h"
#include "qemu/module.h"

#define TYPE_DUMMY_DEVICE "dummy-device"
OBJECT_DECLARE_SIMPLE_TYPE(DummyDeviceState, DUMMY_DEVICE)

struct DummyDeviceState {
  SysBusDevice parent_obj;
  MemoryRegion iomem;
};

static uint64_t dummy_read(void *opaque, hwaddr offset, unsigned size) {
  // cppcheck-suppress unknownMacro
  qemu_log_mask(LOG_GUEST_ERROR,
                "%s: Unimplemented read at offset 0x%" PRIx64 "\n", __func__,
                (uint64_t)offset);
  return 0xdeadbeef;
}

static void dummy_write(void *opaque, hwaddr offset, uint64_t value,
                        unsigned size) {
  // cppcheck-suppress unknownMacro
  qemu_log_mask(LOG_GUEST_ERROR,
                "%s: Unimplemented write at offset 0x%" PRIx64
                " (value: 0x%" PRIx64 ")\n",
                __func__, (uint64_t)offset, value);
}

static const MemoryRegionOps dummy_ops = {
    .read = dummy_read,
    .write = dummy_write,
    .endianness = DEVICE_NATIVE_ENDIAN,
    .valid =
        {
            .min_access_size = 1,
            .max_access_size = 8,
        },
};

static void dummy_init(Object *obj) {
  DummyDeviceState *s = DUMMY_DEVICE(obj);
  SysBusDevice *sbd = SYS_BUS_DEVICE(obj);

  memory_region_init_io(&s->iomem, obj, &dummy_ops, s, "dummy-regs", 0x1000);
  sysbus_init_mmio(sbd, &s->iomem);
}

static void dummy_class_init(ObjectClass *klass, const void *data) {
  DeviceClass *dc = DEVICE_CLASS(klass);
  dc->user_creatable = true;
}

static const TypeInfo dummy_info = {
    .name = TYPE_DUMMY_DEVICE,
    .parent = TYPE_SYS_BUS_DEVICE,
    .instance_size = sizeof(DummyDeviceState),
    .instance_init = dummy_init,
    .class_init = dummy_class_init,
};

static void dummy_register_types(void) { type_register_static(&dummy_info); }

type_init(dummy_register_types);
module_obj(TYPE_DUMMY_DEVICE);

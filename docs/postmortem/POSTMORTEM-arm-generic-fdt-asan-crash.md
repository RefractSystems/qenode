# Post-Mortem: ASan Crash in arm-generic-fdt (sysbus_parse_reg)

## Status
**Fixed**: 2026-04-20  
**Impact**: High (Prevention of QEMU startup crashes under ASan/UBSan)  
**Root Cause**: NULL pointer dereference in the out-of-tree `arm-generic-fdt` patch set.

---

## Technical Context
During CI runs using AddressSanitizer (ASan), QEMU would crash immediately upon parsing the Flattened Device Tree (FDT). The crash occurred in `sysbus_parse_reg`, a function introduced by the `arm-generic-fdt` patch series.

### The Failure Logic
The `arm-generic-fdt` series allows QEMU to dynamically instantiate and wire devices based on a DTB. When it encounters a node with a `reg` property, it attempts to map those memory regions into the system memory map.

The original code in `third_party/qemu/hw/core/sysbus.c` (added via patch) looked like this:

```c
for (i = 0; i < reg.n; ++i) {
    // ... parent resolution ...
    memory_region_add_subregion_overlap(mr_parent, reg.a[i],
                             sysbus_mmio_get_region(SYS_BUS_DEVICE(obj), i),
                             reg.p[i]);
}
```

The bug was two-fold:
1. **Unsafe Casting**: It used `SYS_BUS_DEVICE(obj)` without verifying that `obj` (the instantiated FDT node) actually inherited from `TYPE_SYS_BUS_DEVICE`.
2. **Missing NULL Check**: It assumed `sysbus_mmio_get_region` would always return a valid `MemoryRegion`. If a DTB node had more `reg` entries than the device had registered MMIO regions (common with virtual boundary devices or custom Rust models like `actuator`), it returned `NULL`.

ASan correctly identified that `memory_region_add_subregion_overlap` then attempted to access `subregion->priority`, causing a crash.

## The Fix
We implemented a hardening patch via `patches/apply_sysbus_asan_fix.py`. This script is executed by `scripts/apply-qemu-patches.sh` after the base `.mbx` patches are applied.

### Injected Logic
```c
    SysBusDevice *sbd = (SysBusDevice *)object_dynamic_cast(OBJECT(obj), TYPE_SYS_BUS_DEVICE);

    if (!sbd) {
        return false; // Skip if it's not actually a SysBusDevice
    }

    for (i = 0; i < reg.n; ++i) {
        // ... (parent logic) ...
        mr = sysbus_mmio_get_region(sbd, i);
        if (mr) {
            memory_region_add_subregion_overlap(mr_parent, reg.a[i],
                                     mr,
                                     reg.p[i]);
        }
    }
```

## Upstreaming Note
**Do NOT attempt to upstream this to mainline QEMU.** 
The code being patched exists only in the `arm-generic-fdt` patch set (Xilinx/Refract fork).

**Action Item**: This fix should be submitted to the maintainers of the `arm-generic-fdt` patch series to ensure future versions of the `.mbx` files include this safety check natively.

---

## Discovery & Verification
- **Discovered by**: Gemini CLI during `make ci-asan` loop.
- **Verification**: Verified via `make ci-local` and manual targeted runs of `test/actuator/smoke_test.sh` inside the `devenv-base` container.

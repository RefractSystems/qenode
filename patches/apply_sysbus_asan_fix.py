import logging
import sys
from pathlib import Path

logger = logging.getLogger(__name__)

qemu_dir = sys.argv[1]
filepath = Path(qemu_dir) / "hw/core/sysbus.c"

if not Path(filepath).exists():
    logger.info(f"File {filepath} not found, skipping.")
    sys.exit(0)

with Path(filepath).open() as f:
    text = f.read()

# Fix: Harden sysbus_parse_reg to prevent NULL pointer dereference under ASan/UBSan
# This happens when FDT 'reg' property has more entries than the device's MMIO regions.

old_logic = """static bool sysbus_parse_reg(FDTGenericMMap *obj, FDTGenericRegPropInfo reg,
                             Error **errp)
{
    int i;

    for (i = 0; i < reg.n; ++i) {
        MemoryRegion *mr_parent = (MemoryRegion *)
            object_dynamic_cast(reg.parents[i], TYPE_MEMORY_REGION);
        if (!mr_parent) {
            /* evil */
            mr_parent = get_system_memory();
        }
        memory_region_add_subregion_overlap(mr_parent, reg.a[i],
                                 sysbus_mmio_get_region(SYS_BUS_DEVICE(obj), i),
                                 reg.p[i]);
    }
    return false;
}"""

new_logic = """static bool sysbus_parse_reg(FDTGenericMMap *obj, FDTGenericRegPropInfo reg,
                             Error **errp)
{
    int i;
    SysBusDevice *sbd = (SysBusDevice *)object_dynamic_cast(OBJECT(obj), TYPE_SYS_BUS_DEVICE);

    if (!sbd) {
        return false;
    }

    for (i = 0; i < reg.n; ++i) {
        MemoryRegion *mr_parent = (MemoryRegion *)
            object_dynamic_cast(reg.parents[i], TYPE_MEMORY_REGION);
        MemoryRegion *mr;

        if (!mr_parent) {
            /* evil */
            mr_parent = get_system_memory();
        }

        mr = sysbus_mmio_get_region(sbd, i);
        if (mr && !mr->container) {
            memory_region_add_subregion_overlap(mr_parent, reg.a[i],
                                     mr,
                                     reg.p[i]);
        }
    }
    return false;
}"""

if old_logic in text:
    text = text.replace(old_logic, new_logic)
    logger.info("Applied sysbus_parse_reg ASan fix.")
else:
    # Check if already applied (partially or fully)
    if "SysBusDevice *sbd =" in text:
        logger.info("sysbus_parse_reg ASan fix already applied.")
    else:
        logger.warning("WARNING: Could not find sysbus_parse_reg to patch!")

with Path(filepath).open("w") as f:
    f.write(text)

import sys
from pathlib import Path

qemu_dir = sys.argv[1]
filepath = Path(qemu_dir) / "hw/core/fdt_generic_util.c"

if not Path(filepath).exists():
    print(f"File {filepath} not found, skipping.")
    sys.exit(0)

with Path(filepath).open() as f:
    text = f.read()

# Fix 1: Fix the bug where #address-cells was read from the current node instead of the parent
text = text.replace(
    "qemu_fdt_getprop_cell_inherited(fdti->fdt, node_path,\n                                            size_prop_name",
    "qemu_fdt_getprop_cell_inherited(fdti->fdt, pnp,\n                                            size_prop_name",
)

# Fix 2 (Task 21.7.1): Harden arm-generic-fdt Bus Assignment
old_bus_logic = """        if (object_dynamic_cast(dev, TYPE_DEVICE)) {
            Object *parent_bus = parent;
            unsigned int depth = 0;

            fdt_debug_np("bus parenting node\\n");
            /* Look for an FDT ancestor that is a Bus.  */
            while (parent_bus && !object_dynamic_cast(parent_bus, TYPE_BUS)) {"""

new_bus_logic = """        if (object_dynamic_cast(dev, TYPE_DEVICE)) {
            Object *parent_bus = parent;
            DeviceClass *dc = DEVICE_GET_CLASS(dev);
            unsigned int depth = 0;

            fdt_debug_np("bus parenting node\\n");

            /* Task 21.7.1: Look for a child bus of the right type first */
            if (parent && object_dynamic_cast(parent, TYPE_DEVICE)) {
                DeviceState *ps = DEVICE(parent);
                BusState *b;
                QLIST_FOREACH(b, &ps->child_bus, sibling) {
                    if (!dc->bus_type || object_dynamic_cast(OBJECT(b), dc->bus_type)) {
                        parent_bus = OBJECT(b);
                        break;
                    }
                }
            }

            /* Look for an FDT ancestor that is a Bus.  */
            while (parent_bus && !object_dynamic_cast(parent_bus, TYPE_BUS)) {"""

if old_bus_logic in text:
    text = text.replace(old_bus_logic, new_bus_logic)
    print("Applied Task 21.7.1 bus assignment hardening.")
else:
    print("Task 21.7.1 bus logic already applied or match not found.")

# Fix 3: Fix 8-byte integer truncation in get_int_be
text = text.replace(
    "return be32_to_cpu(*((uint64_t *)p));",
    "return be64_to_cpu(*((uint64_t *)p));",
)

# Fix 4: Remove legacy_reset usage (removed in QEMU 9.0+)
# Also remove the now-unused 'dc' variable definition
text = text.replace(
    "        DeviceClass *dc = DEVICE_GET_CLASS(dev);\n        const char *short_name = strrchr(node_path, '/') + 1;",
    "        const char *short_name = strrchr(node_path, '/') + 1;",
)

old_realize_logic = """            object_property_set_bool(OBJECT(dev), "realized", true,
                                     &error_fatal);
            if (dc->legacy_reset) {
                qemu_register_reset((void (*)(void *))dc->legacy_reset,
                                    dev);
            }
        }"""

new_realize_logic = """            object_property_set_bool(OBJECT(dev), "realized", true,
                                     &error_fatal);
        }"""

text = text.replace(old_realize_logic, new_realize_logic)

# Fix 5: Fix void pointer arithmetic in fdt_init_qdev_array_prop
text = text.replace("        prop_value += elem_len;", "        prop_value = (const uint8_t *)prop_value + elem_len;")

# Fix 6: Fix memory leak in fdt_init_parent_node (strdup not freed)
old_parenting_logic = """    } else if (parent) {
        fdt_debug_np("parenting node\\n");
        object_property_add_child(OBJECT(parent),
                              strdup(strrchr(node_path, '/') + 1),
                              OBJECT(dev));"""

new_parenting_logic = """    } else if (parent) {
        char *name;
        fdt_debug_np("parenting node\\n");
        name = g_strdup(strrchr(node_path, '/') + 1);
        object_property_add_child(OBJECT(parent), name, OBJECT(dev));
        g_free(name);"""

text = text.replace(old_parenting_logic, new_parenting_logic)

# Fix 7: Fix memory leak in fdt_init_set_opaque (strdup not freed if overwritten)
# Actually the logic there was adding new ones if not found.
# But it was using strdup.
text = text.replace("dp->node_path = strdup(node_path);", "dp->node_path = g_strdup(node_path);")

with Path(filepath).open("w") as f:
    f.write(text)
print("Finished applying fdt_generic_util fixes.")

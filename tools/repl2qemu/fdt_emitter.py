import os
import subprocess
import sys

from .parser import ReplPlatform

# Mapping from Renode peripheral types to QEMU device tree compatible strings (QOM type names)
COMPAT_MAP = {
    "UART.STM32_UART": "stm32-usart",
    "UART.PL011": "pl011",
    "UART.Cadence_UART": "cadence_uart",
    "IRQControllers.NVIC": "armv7m_nvic",
    "IRQControllers.ARM_GenericInterruptController": "arm_gic",
    "Timers.ARM_GenericTimer": "armv8-timer",
    "Timers.ARM_PrivateTimer": "arm_mptimer",
    "Miscellaneous.ArmSnoopControlUnit": "a9mpcore_priv", # Or similar depending on exact board
    "CPU.CortexM": "arm,cortex-m-cpu",  # Actually handled specially
    "Memory.MappedMemory": "qemu-memory-region",
}

class FdtEmitter:
    def __init__(self, platform: ReplPlatform):
        self.platform = platform

    def _parse_addr(self, addr_str: str) -> tuple[int, int]:
        """Parses address string '0x60000000' or '<0x40011000, +0x100>'."""
        if not addr_str or addr_str.lower() == 'none' or not any(c.isdigit() for c in addr_str):
            return 0, 0

        addr_str = addr_str.strip()
        if addr_str.startswith('<'):
            # <0x40011000, +0x100>
            parts = addr_str.strip('<>').split(',')
            base = int(parts[0].strip(), 16)
            size_part = parts[1].strip()
            if size_part.startswith('+'):
                size_part = size_part[1:]
            size = int(size_part, 16)
            return base, size
        else:
            try:
                # 0x60000000 (size usually in properties)
                return int(addr_str, 16), 0
            except ValueError:
                return 0, 0

    def generate_dts(self) -> str:
        lines = []
        lines.append("/dts-v1/;")
        lines.append("")
        lines.append("/ {")
        lines.append('    model = "virtmcu-dynamic-machine";')
        lines.append('    compatible = "arm,generic-fdt";')
        lines.append("    #address-cells = <2>;")
        lines.append("    #size-cells = <2>;")
        lines.append("")
        lines.append("    qemu_sysmem: qemu_sysmem {")
        lines.append('        compatible = "qemu:system-memory";')
        lines.append("        phandle = <0x01>;")
        lines.append("    };")
        lines.append("")

        # cpus node
        lines.append("    cpus {")
        lines.append("        #address-cells = <1>;")
        lines.append("        #size-cells = <0>;")

        cpu_index = 0
        for dev in self.platform.devices:
            if dev.type_name in ["CPU.CortexM", "CPU.ARMv8A", "CPU.ARMv7A"]:
                # E.g., cpuType: "cortex-a15"
                cpu_type = dev.properties.get("cpuType", "cortex-m3")
                lines.append(f"        {dev.name}@{cpu_index} {{")
                lines.append('            device_type = "cpu";')
                lines.append(f'            compatible = "{cpu_type}-arm-cpu";')
                lines.append(f'            reg = <{cpu_index}>;')
                lines.append("            memory = <0x01>;")
                lines.append("        };")
                cpu_index += 1
        lines.append("    };")
        lines.append("")

        for dev in self.platform.devices:
            if dev.type_name in ["CPU.CortexM", "CPU.ARMv8A", "CPU.ARMv7A"]:
                continue # Handled above

            base, size = self._parse_addr(dev.address_str)

            if dev.type_name == "Memory.MappedMemory":
                if "size" in dev.properties:
                    size = int(dev.properties["size"], 16)

                # Emit memory node (name must start with 'memory@' for QEMU arm-generic-fdt)
                lines.append(f"    memory@{base:x} {{")
                lines.append('        compatible = "qemu-memory-region";')
                lines.append("        qemu,ram = <0x01>;")
                lines.append("        container = <0x01>;")
                lines.append(f"        reg = <0x0 0x{base:x} 0x0 0x{size:x}>;")
                lines.append("    };")

            elif dev.type_name in COMPAT_MAP:
                if size == 0:
                    size = 0x1000 # Default size if not provided

                lines.append(f"    {dev.name}@{base:x} {{")
                lines.append(f'        compatible = "{COMPAT_MAP[dev.type_name]}";')

                # armv8-timer doesn't have MMIO registers in QEMU DTS
                if COMPAT_MAP[dev.type_name] != "armv8-timer":
                    # Some devices might just be at 0x0 genuinely (like flash), but for others,
                    # if base == 0 and we couldn't parse it, we shouldn't emit a zeroed reg.
                    lines.append(f"        reg = <0x0 0x{base:x} 0x0 0x{size:x}>;")

                if dev.type_name.startswith("UART"):
                    lines.append("        chardev = <0x00>;")

                # We could map interrupts here
                if dev.interrupts:
                    # simplistic mapping for now: assume target is nvic and ignore the index format specifics
                    target_irq = dev.interrupts[0].target_range
                    if '-' not in target_irq:
                        lines.append(f"        interrupts = <0 {target_irq} 4>;")

                lines.append("        container = <0x01>;")
                lines.append("    };")
            else:
                print(f"Warning: no QEMU mapping for Renode type '{dev.type_name}' (device '{dev.name}' skipped)", file=sys.stderr)

        lines.append("};")
        return "\n".join(lines)

def compile_dtb(dts_content: str, out_path: str) -> bool:
    """Compiles the DTS string into a DTB file using dtc."""
    dts_path = out_path + ".dts"
    try:
        with open(dts_path, "w") as f:
            f.write(dts_content)

        subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", out_path, dts_path], check=True, capture_output=True)
        return True
    except subprocess.CalledProcessError as e:
        print(f"Error compiling DTB: {e.stderr.decode()}", file=sys.stderr)
        return False
    finally:
        if os.path.exists(dts_path):
            os.unlink(dts_path)


if __name__ == "__main__":
    import sys

    from .parser import parse_repl
    filename = sys.argv[1] if len(sys.argv) > 1 else "third_party/renode/platforms/boards/cortex_a53_virtio.repl"
    with open(filename, "r") as f:
        plat = parse_repl(f.read())

    emitter = FdtEmitter(plat)
    dts = emitter.generate_dts()
    print(dts)
    compile_dtb(dts, "test_out.dtb")

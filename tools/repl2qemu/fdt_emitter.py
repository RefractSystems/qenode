import os
import subprocess
import sys
from typing import Dict, List, Tuple

from .parser import ReplDevice, ReplPlatform

# Mapping from Renode peripheral types to QEMU device tree compatible strings (QOM type names)
COMPAT_MAP = {
    "UART.STM32_UART": "stm32-usart",
    "UART.PL011": "pl011",
    "UART.Cadence_UART": "cadence_uart",
    "IRQControllers.NVIC": "armv7m_nvic",
    "IRQControllers.ARM_GenericInterruptController": "arm_gic",
    "IRQControllers.GIC": "arm_gic",
    "Timers.ARM_GenericTimer": "armv8-timer",
    "Timers.ARM_PrivateTimer": "arm_mptimer",
    "Miscellaneous.ArmSnoopControlUnit": "a9mpcore_priv",
    "CPU.CortexM": "arm,cortex-m-cpu",
    "CPU.CortexA": "arm,cortex-a-cpu",
    "CPU.ARMv7A": "arm,cortex-a-cpu",
    "CPU.RISCV64": "riscv",
    "Memory.MappedMemory": "qemu-memory-region",
    "RemotePort.Peripheral": "remote-port-bridge",
    "Network.IMX_FEC": "imx.fec",
    "Network.LAN9118": "lan9118",
    "SPI.PL022": "pl022",
}

# Devices that act as interrupt controllers
INT_CONTROLLERS = {
    "IRQControllers.NVIC",
    "IRQControllers.ARM_GenericInterruptController",
    "IRQControllers.GIC",
}


class FdtEmitter:
    def __init__(self, platform: ReplPlatform):
        self.platform = platform
        self.arch = self._detect_arch()
        self.phandles: Dict[str, int] = {}
        self.next_phandle = 1
        self._assign_phandles()

    def _detect_arch(self) -> str:
        for dev in self.platform.devices:
            if "RISCV" in dev.type_name.upper():
                return "riscv"
        return "arm"

    def _assign_phandles(self):
        # Always have a sysmem phandle
        self.phandles["qemu_sysmem"] = self.next_phandle
        self.next_phandle += 1

        # Assign phandles to all devices that might be referenced
        for dev in self.platform.devices:
            self.phandles[dev.name] = self.next_phandle
            self.next_phandle += 1

    def _get_phandle(self, name: str) -> int:
        return self.phandles.get(name, 0)

    def _parse_addr(self, addr_str: str) -> Tuple[int, int]:
        """Parses address string '0x60000000' or '<0x40011000, +0x100>'."""
        if not addr_str or addr_str.lower() == "none" or not any(c.isdigit() for c in addr_str):
            return 0, 0

        addr_str = addr_str.strip()
        if addr_str.startswith("<"):
            parts = addr_str.strip("<>").split(",")
            base = int(parts[0].strip(), 16)
            size_part = parts[1].strip()
            if size_part.startswith("+"):
                size_part = size_part[1:]
            size = int(size_part, 16)
            return base, size
        else:
            try:
                return int(addr_str, 16), 0
            except ValueError:
                return 0, 0

    def generate_dts(self) -> str:
        lines = []
        lines.append("/dts-v1/;")
        lines.append("")
        lines.append("/ {")
        lines.append('    model = "virtmcu-dynamic-machine";')
        if self.arch == "riscv":
            lines.append('    compatible = "riscv-virtio";')
        else:
            lines.append('    compatible = "arm,generic-fdt";')
        lines.append("    #address-cells = <2>;")
        lines.append("    #size-cells = <2>;")
        lines.append("")
        lines.append("    qemu_sysmem: qemu_sysmem {")
        lines.append('        compatible = "qemu:system-memory";')
        lines.append(f"        phandle = <{self._get_phandle('qemu_sysmem')}>;")
        lines.append("    };")
        lines.append("")

        # cpus node
        lines.append("    cpus {")
        lines.append("        #address-cells = <1>;")
        lines.append("        #size-cells = <0>;")
        if self.arch == "riscv":
            lines.append("        timebase-frequency = <10000000>;")

        cpu_index = 0
        for dev in self.platform.devices:
            if "CPU" in dev.type_name:
                cpu_type = dev.properties.get("cpuType", "cortex-m3" if self.arch == "arm" else "rv64")
                lines.append(f"        {dev.name}@{cpu_index} {{")
                lines.append('            device_type = "cpu";')
                if self.arch == "riscv":
                    lines.append(f'            compatible = "riscv,{cpu_type}";')
                    lines.append(f'            riscv,isa = "{dev.properties.get("isa", "rv64imafdc")}";')
                    lines.append(f'            mmu-type = "{dev.properties.get("mmu-type", "riscv,sv48")}";')
                else:
                    lines.append(f'            compatible = "{cpu_type}-arm-cpu";')
                lines.append(f"            reg = <{cpu_index}>;")
                lines.append(f"            memory = <{self._get_phandle('qemu_sysmem')}>;")
                if self.arch == "riscv":
                    lines.append(f"            {dev.name}_intc: interrupt-controller {{")
                    lines.append("                #interrupt-cells = <1>;")
                    lines.append("                interrupt-controller;")
                    lines.append('                compatible = "riscv,cpu-intc";')
                    lines.append(f"                phandle = <{self.next_phandle}>;")
                    self.phandles[f"{dev.name}_intc"] = self.next_phandle
                    self.next_phandle += 1
                    lines.append("            };")
                lines.append("        };")
                cpu_index += 1
        lines.append("    };")
        lines.append("")

        # Pre-process children
        children_by_parent = {}
        for dev in self.platform.devices:
            if dev.parent:
                if dev.parent not in children_by_parent:
                    children_by_parent[dev.parent] = []
                children_by_parent[dev.parent].append(dev)

        for dev in self.platform.devices:
            if "CPU" in dev.type_name or dev.parent:
                continue

            lines.extend(self._emit_device(dev, children_by_parent))

        lines.append("};")
        return "\n".join(lines)

    def _emit_device(
        self, dev: ReplDevice, children_by_parent: Dict[str, List[ReplDevice]], indent: str = "    "
    ) -> List[str]:
        lines = []
        base, size = self._parse_addr(dev.address_str)

        if dev.type_name == "Memory.MappedMemory":
            if "size" in dev.properties:
                size_val = dev.properties["size"]
                size = int(size_val, 16) if isinstance(size_val, str) else int(size_val)

            lines.append(f"{indent}memory@{base:x} {{")
            lines.append(f'{indent}    compatible = "qemu-memory-region";')
            lines.append(f"{indent}    qemu,ram = <{self._get_phandle('qemu_sysmem')}>;")
            lines.append(f"{indent}    container = <{self._get_phandle('qemu_sysmem')}>;")
            base_hi, base_lo = (base >> 32) & 0xFFFFFFFF, base & 0xFFFFFFFF
            size_hi, size_lo = (size >> 32) & 0xFFFFFFFF, size & 0xFFFFFFFF
            lines.append(f"{indent}    reg = <0x{base_hi:x} 0x{base_lo:x} 0x{size_hi:x} 0x{size_lo:x}>;")
            lines.append(f"{indent}}};")
            return lines

        is_native = dev.type_name not in COMPAT_MAP and "." not in dev.type_name
        if dev.type_name not in COMPAT_MAP and not is_native:
            print(
                f"Warning: no QEMU mapping for Renode type '{dev.type_name}' (device '{dev.name}' skipped)",
                file=sys.stderr,
            )
            return []

        compat_str = dev.type_name if is_native else COMPAT_MAP[dev.type_name]

        # If it's a child, base might be small (reg index)
        lines.append(f"{indent}{dev.name}@{base:x} {{")
        lines.append(f'{indent}    compatible = "{compat_str}";')
        lines.append(f"{indent}    phandle = <{self._get_phandle(dev.name)}>;")

        # Reg property handling
        if dev.parent:
            # Child of a bus (like SPI)
            lines.append(f"{indent}    reg = <{base}>;")
        elif compat_str != "armv8-timer":
            base_hi, base_lo = (base >> 32) & 0xFFFFFFFF, base & 0xFFFFFFFF
            if size == 0:
                size = 0x1000
            size_hi, size_lo = (size >> 32) & 0xFFFFFFFF, size & 0xFFFFFFFF
            lines.append(f"{indent}    reg = <0x{base_hi:x} 0x{base_lo:x} 0x{size_hi:x} 0x{size_lo:x}>;")

        if dev.type_name.startswith("UART") or compat_str == "pl011" or "chardev" in dev.properties:
            lines.append(f"{indent}    chardev = <0x00>;")

        if dev.interrupts:
            # Find interrupt parent
            target_name = dev.interrupts[0].target_device
            parent_phandle = self._get_phandle(target_name)
            if parent_phandle:
                lines.append(f"{indent}    interrupt-parent = <{parent_phandle}>;")

            # simplistic mapping: SPI, ID, level/edge
            irqs = []
            for irq in dev.interrupts:
                target_irq = irq.target_range
                if "-" not in target_irq:
                    # TODO: Detect if parent is GIC or NVIC
                    # GIC expects <type id flags>, NVIC expects <id>
                    is_gic = any(
                        ic in target_name.upper() or (i_dev.name == target_name and "GIC" in i_dev.type_name.upper())
                        for ic in ["GIC", "DISTRIBUTOR"]
                        for i_dev in self.platform.devices
                    )

                    if is_gic:
                        # Renode often uses SPI index (0+) for GIC interrupts.
                        # QEMU's GIC expects absolute IRQ number (32+) for SPIs.
                        irq_num = int(target_irq)
                        if irq_num < 32:
                            irq_num += 32
                        irqs.append(f"<0 {irq_num} 4>")
                    else:
                        irqs.append(f"<{target_irq}>")
            if irqs:
                lines.append(f"{indent}    interrupts = {', '.join(irqs)};")

        if dev.type_name in INT_CONTROLLERS:
            lines.append(f"{indent}    interrupt-controller;")
            if "NVIC" in dev.type_name:
                lines.append(f"{indent}    #interrupt-cells = <1>;")
            else:
                lines.append(f"{indent}    #interrupt-cells = <3>;")

        # SPI Bus properties
        if dev.type_name.startswith("SPI"):
            lines.append(f"{indent}    #address-cells = <1>;")
            lines.append(f"{indent}    #size-cells = <0>;")

        for k, v in dev.properties.items():
            if k in ["size", "cpuType", "isa", "mmu-type", "chardev"]:
                if k == "size" and compat_str == "mmio-socket-bridge":
                    # Backward compatibility: map 'size' to 'region-size'
                    if "region-size" not in dev.properties:
                        val = v if isinstance(v, int) else int(v, 16)
                        lines.append(f"{indent}    region-size = <0x{val:x}>;")
                continue
            if k == "address" and compat_str == "mmio-socket-bridge":
                # Backward compatibility: map 'address' to 'base-addr'
                if "base-addr" not in dev.properties:
                    v_hi, v_lo = (v >> 32) & 0xFFFFFFFF, v & 0xFFFFFFFF
                    lines.append(f"{indent}    base-addr = <0x{v_hi:x} 0x{v_lo:x}>;")
                continue

            # Special handling for known 64-bit properties
            if k == "base-addr" and isinstance(v, int):
                v_hi, v_lo = (v >> 32) & 0xFFFFFFFF, v & 0xFFFFFFFF
                lines.append(f"{indent}    {k} = <0x{v_hi:x} 0x{v_lo:x}>;")
            elif isinstance(v, bool):
                if v:
                    lines.append(f"{indent}    {k};")
            elif isinstance(v, int):
                lines.append(f"{indent}    {k} = <0x{v:x}>;")
            else:
                lines.append(f'{indent}    {k} = "{v}";')

        if not dev.parent:
            lines.append(f"{indent}    container = <{self._get_phandle('qemu_sysmem')}>;")

        # Emit children
        if dev.name in children_by_parent:
            for child in children_by_parent[dev.name]:
                lines.extend(self._emit_device(child, children_by_parent, indent + "    "))

        lines.append(f"{indent}}};")
        return lines


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

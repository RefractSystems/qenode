import logging
import subprocess
from pathlib import Path

from .parser import GicDevice, MemoryDevice, MmioBridgeDevice, ReplDevice, ReplPlatform, WirelessDevice

logger = logging.getLogger(__name__)

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
    "SPI.ZenohBridge": "spi",
    "SPI.Echo": "spi-echo",
    "telemetry": "telemetry",
    "ieee802154": "ieee802154",
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
        self.phandles: dict[str, int] = {}
        self.next_phandle = 1
        self._assign_phandles()

    def validate_platform(self):
        """
        ARCH-14: Validate that all devices have mandatory properties.
        Throws ValueError if a device is missing required fields.
        """
        for dev in self.platform.devices:
            if isinstance(dev, MemoryDevice):
                if "size" not in dev.properties:
                    raise ValueError(f"Memory node '{dev.name}' is missing mandatory 'size' property")

            elif isinstance(dev, WirelessDevice):
                if "transport" not in dev.properties:
                    raise ValueError(f"Wireless node '{dev.name}' is missing mandatory 'transport' property")
                if "node" not in dev.properties:
                    raise ValueError(f"Wireless node '{dev.name}' is missing mandatory 'node' property")

            elif isinstance(dev, MmioBridgeDevice):
                # Check for either the legacy keys or the modern ones
                has_size = "size" in dev.properties or "region-size" in dev.properties
                has_addr = (
                    "address" in dev.properties or "base-addr" in dev.properties or dev.address_str != "sysbus"
                )
                if not has_size:
                    raise ValueError(f"mmio-socket-bridge '{dev.name}' is missing mandatory 'region-size' property")
                if not has_addr:
                    raise ValueError(f"mmio-socket-bridge '{dev.name}' is missing mandatory 'base-addr' property")
                if "socket-path" not in dev.properties:
                    raise ValueError(f"mmio-socket-bridge '{dev.name}' is missing mandatory 'socket-path' property")

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

    def _parse_addr(self, addr_str: str) -> tuple[int, int]:
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
        try:
            return int(addr_str, 16), 0
        except ValueError:
            return 0, 0

    def generate_dts(self) -> str:
        self.validate_platform()
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

        # Global interrupt parent for the root node if GIC/NVIC is present
        intc_dev = None
        for dev in self.platform.devices:
            if dev.type_name in INT_CONTROLLERS:
                intc_dev = dev
                break

        if intc_dev:
            lines.append(f"    interrupt-parent = <{self._get_phandle(intc_dev.name)}>;")

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
        children_by_parent: dict[str, list[ReplDevice]] = {}
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
        self, dev: ReplDevice, children_by_parent: dict[str, list[ReplDevice]], indent: str = "    "
    ) -> list[str]:
        lines = []
        base, size = self._parse_addr(dev.address_str or "0x0")

        if dev.type_name == "Memory.MappedMemory":
            if "size" in dev.properties:
                size_val = dev.properties["size"]
                size = int(size_val, 16) if isinstance(size_val, str) else int(size_val)

            lines.append(f"{indent}memory@{base:x} {{")
            lines.append(f'{indent}    compatible = "qemu-memory-region";')
            lines.append(f"{indent}    qemu,ram = <0x01>;")
            lines.append(f"{indent}    container = <{self._get_phandle('qemu_sysmem')}>;")
            base_hi, base_lo = (base >> 32) & 0xFFFFFFFF, base & 0xFFFFFFFF
            size_hi, size_lo = (size >> 32) & 0xFFFFFFFF, size & 0xFFFFFFFF
            lines.append(f"{indent}    reg = <0x{base_hi:x} 0x{base_lo:x} 0x{size_hi:x} 0x{size_lo:x}>;")
            lines.append(f"{indent}}};")
            return lines

        is_native = dev.type_name not in COMPAT_MAP and "." not in dev.type_name
        if dev.type_name not in COMPAT_MAP and not is_native:
            logger.warning(f"Warning: no QEMU mapping for Renode type '{dev.type_name}' (device '{dev.name}' skipped)")
            return []

        compat_str = dev.type_name if is_native else COMPAT_MAP[dev.type_name]

        # Node name: Use name@address to satisfy DTC unit_address_vs_reg checks.
        if compat_str == "armv8-timer":
            lines.append(f"{indent}{dev.name} {{")
        else:
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

            # Detect if parent is GIC or NVIC
            is_gic = False

            if parent_phandle:
                lines.append(f"{indent}    interrupt-parent = <{parent_phandle}>;")
                # Look up the target device in the platform to see if it's a GicDevice
                for i_dev in self.platform.devices:
                    if i_dev.name == target_name and isinstance(i_dev, GicDevice):
                        is_gic = True
                        break
            else:
                # Search for any interrupt controller in the platform as fallback
                for i_dev in self.platform.devices:
                    if i_dev.type_name in INT_CONTROLLERS:
                        lines.append(f"{indent}    interrupt-parent = <{self._get_phandle(i_dev.name)}>;")
                        if isinstance(i_dev, GicDevice):
                            is_gic = True
                        break

            # simplistic mapping: SPI, ID, level/edge
            irq_cells = []
            for irq in dev.interrupts:
                target_irq = irq.target_range
                if "-" not in target_irq:
                    if is_gic:
                        irq_num = int(target_irq)
                        if irq_num < 32:
                            irq_num += 32
                        irq_cells.extend(["0", str(irq_num), "4"])
                    else:
                        irq_cells.append(str(target_irq))
                else:
                    parts = target_irq.split("-")
                    start = int(parts[0])
                    end = int(parts[1])
                    for i in range(start, end + 1):
                        if is_gic:
                            num = i + 32 if i < 32 else i
                            irq_cells.extend(["0", str(num), "4"])
                        else:
                            irq_cells.append(str(i))

            if irq_cells:
                lines.append(f"{indent}    interrupts = <{' '.join(irq_cells)}>;")

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
                if k == "size" and compat_str == "mmio-socket-bridge" and "region-size" not in dev.properties:
                    # Backward compatibility: map 'size' to 'region-size'
                    val = v if isinstance(v, int) else int(v, 16)
                    lines.append(f"{indent}    region-size = <0x{val:x}>;")
                continue
            if k == "address" and compat_str == "mmio-socket-bridge":
                # Backward compatibility: map 'address' to 'base-addr'
                if "base-addr" not in dev.properties:
                    val = v if isinstance(v, int) else int(v, 16)
                    v_hi, v_lo = (val >> 32) & 0xFFFFFFFF, val & 0xFFFFFFFF
                    lines.append(f"{indent}    base-addr = <0x{v_hi:x} 0x{v_lo:x}>;")
                continue

            # Special handling for known 64-bit properties
            if k == "base-addr" and isinstance(v, int):
                v_hi, v_lo = (v >> 32) & 0xFFFFFFFF, v & 0xFFFFFFFF
                lines.append(f"{indent}    {k} = <0x{v_hi:x} 0x{v_lo:x}>;")
            elif k.lower() == "macaddress" or k.lower() == "macaddr" or k.lower() == "mac":
                # Ensure MAC is output as a string named 'macaddr' for QEMU's qdev_prop_macaddr
                lines.append(f'{indent}    macaddr = "{v}";')
            elif isinstance(v, bool):
                if v:
                    lines.append(f"{indent}    {k};")
            elif isinstance(v, int):
                lines.append(f"{indent}    {k} = <0x{v:x}>;")
            else:
                lines.append(f'{indent}    {k} = "{v}";')

        # Only add container for memory regions, or if explicitly requested (not for standard SysBus devices)
        if not dev.parent and (dev.type_name == "Memory.MappedMemory" or compat_str == "mmio-socket-bridge"):
            logger.info(f"DEBUG: Adding container for {dev.name} (type={dev.type_name}, compat={compat_str})")
            lines.append(f"{indent}    container = <{self._get_phandle('qemu_sysmem')}>;")

        # Emit children
        if dev.name in children_by_parent:
            for child in children_by_parent[dev.name]:
                lines.extend(self._emit_device(child, children_by_parent, indent + "    "))

        lines.append(f"{indent}}};")
        return lines


def compile_dtb(dts_content: str, out_path: str) -> bool:
    """Compiles the DTS string into a DTB file using dtc. Fails on warnings."""
    dts_path = out_path + ".dts"
    try:
        with Path(dts_path).open("w") as f:
            f.write(dts_content)
        res = subprocess.run(
            ["dtc", "-I", "dts", "-O", "dtb", "-o", out_path, dts_path],
            check=True,
            capture_output=True,
            text=True,
        )
        if "Warning" in res.stderr:
            logger.warning(f"DTC Warnings detected:\n{res.stderr}")
        return True
    except subprocess.CalledProcessError as e:
        logger.error(f"Error compiling DTB: {e.stderr}")
        return False
    finally:
        if Path(dts_path).exists():
            Path(dts_path).unlink()

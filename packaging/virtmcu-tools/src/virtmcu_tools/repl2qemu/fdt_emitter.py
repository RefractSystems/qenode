import logging
import subprocess
import sys
from pathlib import Path

from .parser import ReplPlatform

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
}

# Devices that act as interrupt controllers
INT_CONTROLLERS = {
    "IRQControllers.NVIC",
    "IRQControllers.ARM_GenericInterruptController",
    "IRQControllers.GIC",
}


class FdtEmitter:
    def __init__(self, platform: ReplPlatform) -> None:
        self.platform = platform
        self.arch = self._detect_arch()
        self.phandles: dict[str, int] = {}
        self.next_phandle = 1
        self._assign_phandles()

    def _detect_arch(self) -> str:
        for dev in self.platform.devices:
            if "RISCV" in dev.type_name.upper():
                return "riscv"
        return "arm"

    def _assign_phandles(self) -> None:
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

        for dev in self.platform.devices:
            if "CPU" in dev.type_name:
                continue

            base, size = self._parse_addr(dev.address_str)

            if dev.type_name == "Memory.MappedMemory":
                if "size" in dev.properties:
                    size_val = dev.properties["size"]
                    size = int(size_val, 16) if isinstance(size_val, str) else int(size_val)

                lines.append(f"    memory@{base:x} {{")
                lines.append('        compatible = "qemu-memory-region";')
                lines.append(f"        qemu,ram = <{self._get_phandle('qemu_sysmem')}>;")
                lines.append(f"        container = <{self._get_phandle('qemu_sysmem')}>;")
                base_hi, base_lo = (base >> 32) & 0xFFFFFFFF, base & 0xFFFFFFFF
                size_hi, size_lo = (size >> 32) & 0xFFFFFFFF, size & 0xFFFFFFFF
                lines.append(f"        reg = <0x{base_hi:x} 0x{base_lo:x} 0x{size_hi:x} 0x{size_lo:x}>;")
                lines.append("    };")

            else:
                is_native = dev.type_name not in COMPAT_MAP and "." not in dev.type_name
                if dev.type_name not in COMPAT_MAP and not is_native:
                    logger.info(
                        f"Warning: no QEMU mapping for Renode type '{dev.type_name}' (device '{dev.name}' skipped)",
                        file=sys.stderr,
                    )
                    continue

                compat_str = dev.type_name if is_native else COMPAT_MAP[dev.type_name]

                if size == 0:
                    size = 0x1000

                lines.append(f"    {dev.name}@{base:x} {{")
                lines.append(f'        compatible = "{compat_str}";')
                lines.append(f"        phandle = <{self._get_phandle(dev.name)}>;")

                if compat_str != "armv8-timer":
                    base_hi, base_lo = (base >> 32) & 0xFFFFFFFF, base & 0xFFFFFFFF
                    size_hi, size_lo = (size >> 32) & 0xFFFFFFFF, size & 0xFFFFFFFF
                    lines.append(f"        reg = <0x{base_hi:x} 0x{base_lo:x} 0x{size_hi:x} 0x{size_lo:x}>;")

                if dev.type_name.startswith("UART") or compat_str == "pl011":
                    lines.append("        chardev = <0x00>;")

                if dev.interrupts:
                    # Find interrupt parent
                    target_name = dev.interrupts[0].target_device
                    parent_phandle = self._get_phandle(target_name)
                    if parent_phandle:
                        lines.append(f"        interrupt-parent = <{parent_phandle}>;")

                    # simplistic mapping: SPI, ID, level/edge
                    irqs = []
                    for irq in dev.interrupts:
                        target_irq = irq.target_range
                        if "-" not in target_irq:
                            # TODO: Detect if parent is GIC or NVIC
                            # GIC expects <type id flags>, NVIC expects <id>
                            is_gic = any(
                                ic in target_name.upper()
                                or (i_dev.name == target_name and "GIC" in i_dev.type_name.upper())
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
                        lines.append(f"        interrupts = {', '.join(irqs)};")

                if dev.type_name in INT_CONTROLLERS:
                    lines.append("        interrupt-controller;")
                    if "NVIC" in dev.type_name:
                        lines.append("        #interrupt-cells = <1>;")
                    else:
                        lines.append("        #interrupt-cells = <3>;")

                for k, v in dev.properties.items():
                    if k in ["size", "cpuType", "isa", "mmu-type"]:
                        continue
                    if isinstance(v, bool):
                        if v:
                            lines.append(f"        {k};")
                    elif isinstance(v, int):
                        lines.append(f"        {k} = <{v}>;")
                    else:
                        lines.append(f'        {k} = "{v}";')

                lines.append(f"        container = <{self._get_phandle('qemu_sysmem')}>;")
                lines.append("    };")

        lines.append("};")
        return "\n".join(lines)


def compile_dtb(dts_content: str, out_path: str) -> bool:
    """Compiles the DTS string into a DTB file using dtc."""
    dts_path = out_path + ".dts"
    try:
        with Path(dts_path).open("w") as f:
            f.write(dts_content)

        import shutil

        dtc_path = shutil.which("dtc")
        if not dtc_path:
            raise RuntimeError("dtc executable not found in PATH")

        subprocess.run([dtc_path, "-I", "dts", "-O", "dtb", "-o", out_path, dts_path], check=True, capture_output=True)
        return True
    except subprocess.CalledProcessError as e:
        logger.error(f"Error compiling DTB: {e.stderr.decode()}")
        return False
    finally:
        if Path(dts_path).exists():
            Path(dts_path).unlink()

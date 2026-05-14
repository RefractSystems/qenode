use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const HEX_RADIX: u32 = 16;
const DEFAULT_REGION_SIZE: u64 = 0x1000;
const QEMU_RAM_FLAG: u32 = 0x01;
const ADDR_SHIFT: u32 = 32;
const ADDR_MASK: u64 = 0xFFFFFFFF;
const DEFAULT_CHARDEV_PHANDLE: u32 = 0x00;
const INTERRUPT_CELLS: u32 = 3;
const GIC_SPI_OFFSET: u64 = 32;
const DEFAULT_INTERRUPT_FLAG: u32 = 4;
const HUB_PHANDLE: u32 = 2;
const QEMU_SYSMEM_PHANDLE: u64 = 1;
const ADDR_CELLS: u32 = 2;
const SIZE_CELLS: u32 = 2;
const INITIAL_NEXT_PHANDLE: u64 = 3;
const IRQ_STRING_PARTS: usize = 2;
const CPU_ADDR_CELLS: u32 = 1;
const CPU_SIZE_CELLS: u32 = 0;

#[cfg(test)]
const EXAMPLE_NODE_ID: u32 = 1;
#[cfg(test)]
const EXAMPLE_ROUTER_ENDPOINT: &str = "tcp/127.0.0.1:7447";
#[cfg(test)]
const EXAMPLE_UART_ADDR: u64 = 0x4006A000;
#[cfg(test)]
const EXAMPLE_UART_IRQ: u64 = 31;
#[cfg(test)]
const EXPECTED_GIC_IRQ: u32 = 63;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct World {
    #[serde(default)]
    pub machine: Option<Machine>,
    #[serde(default)]
    pub peripherals: Vec<Peripheral>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Machine {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub machine_type: Option<String>,
    #[serde(default)]
    pub cpus: Vec<Cpu>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Cpu {
    pub name: String,
    #[serde(rename = "type")]
    pub cpu_type: String,
    pub memory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Peripheral {
    pub name: String,
    #[serde(alias = "renode_type")]
    #[serde(rename = "type")]
    pub periph_type: Option<String>,
    pub address: Option<serde_yaml::Value>,
    pub properties: Option<HashMap<String, serde_yaml::Value>>,
    pub interrupts: Option<Vec<serde_yaml::Value>>,
    pub parent: Option<String>,
    pub container: Option<String>,
}

#[derive(Debug)]
pub struct YamlPlatform {
    pub dts_content: String,
    pub cli_args: Vec<String>,
    pub has_clock: bool,
}

fn parse_addr(val: &serde_yaml::Value) -> u64 {
    match val {
        serde_yaml::Value::Number(n) => n.as_u64().expect("YAML number should be parseable as u64"),
        serde_yaml::Value::String(s) => {
            let s = s.trim();
            if s == "none" || s == "sysbus" {
                return 0;
            }
            if let Some(stripped) = s.strip_prefix("0x") {
                u64::from_str_radix(stripped, HEX_RADIX).expect("Invalid hex string for address")
            } else {
                s.parse::<u64>().expect("Invalid decimal string for address")
            }
        }
        _ => unreachable!("Address must be a number or string"),
    }
}

fn emit_device(
    p: &Peripheral,
    children_by_parent: &HashMap<String, Vec<Peripheral>>,
    phandles: &HashMap<String, u64>,
    indent: &str,
    dts: &mut String,
    cli_args: &mut Vec<String>,
    endpoint: Option<&str>,
) {
    let addr = if let Some(a) = &p.address { parse_addr(a) } else { 0 };
    let p_type = p.periph_type.as_deref().unwrap_or("Unknown");

    if p_type == "chardev" {
        let node = if let Some(props) = &p.properties {
            if let Some(serde_yaml::Value::String(n)) = props.get("node") {
                n.as_str()
            } else {
                "0"
            }
        } else {
            "0"
        };

        let topic = if let Some(props) = &p.properties {
            if let Some(serde_yaml::Value::String(t)) = props.get("topic") {
                format!(",topic={}", t)
            } else {
                "".to_string()
            }
        } else {
            "".to_string()
        };

        let router_arg =
            if let Some(ep) = endpoint { format!(",router={}", ep) } else { "".to_string() };

        cli_args.push("-chardev".to_string());
        cli_args.push(format!(
            "virtmcu,id=chr_{},node={},transport=zenoh{}{}",
            p.name, node, router_arg, topic
        ));
        return; // chardevs don't emit DT nodes
    }

    if p_type == "Memory.MappedMemory" {
        let size = if let Some(props) = &p.properties {
            if let Some(s) = props.get("size") {
                parse_addr(s)
            } else {
                DEFAULT_REGION_SIZE
            }
        } else {
            DEFAULT_REGION_SIZE
        };

        dts.push_str(&format!("{}memory@{:x} {{\n", indent, addr));
        dts.push_str(&format!("{}    compatible = \"qemu-memory-region\";\n", indent));
        dts.push_str(&format!("{}    qemu,ram = <0x{:x}>;\n", indent, QEMU_RAM_FLAG));
        dts.push_str(&format!("{}    container = <{}>;\n", indent, QEMU_SYSMEM_PHANDLE));

        let addr_hi = addr >> ADDR_SHIFT;
        let addr_lo = addr & ADDR_MASK;
        let size_hi = size >> ADDR_SHIFT;
        let size_lo = size & ADDR_MASK;

        dts.push_str(&format!(
            "{}    reg = <0x{:x} 0x{:x} 0x{:x} 0x{:x}>;\n",
            indent, addr_hi, addr_lo, size_hi, size_lo
        ));
        dts.push_str(&format!("{}}};\n", indent));
        return;
    }

    let compat = match p_type {
        "UART.PL011" => "pl011",
        "IRQControllers.ARM_GenericInterruptController" => "arm_gic",
        "actuator" => "actuator",
        "ieee802154" => "ieee802154",
        "mmio-socket-bridge" => "mmio-socket-bridge",
        "SPI.PL022" => "pl022",
        "spi-echo" => "spi-echo",
        "SPI.ZenohBridge" => "spi",
        _ => p_type,
    };

    dts.push_str(&format!("{}{}@{:x} {{\n", indent, p.name, addr));
    dts.push_str(&format!("{}    compatible = \"{}\";\n", indent, compat));
    dts.push_str(&format!(
        "{}    phandle = <{}>;\n",
        indent,
        phandles.get(&p.name).expect("Phandle must be present")
    ));

    let addr_hi = addr >> ADDR_SHIFT;
    let addr_lo = addr & ADDR_MASK;

    if p.parent.is_none() {
        let mut size = DEFAULT_REGION_SIZE;
        if let Some(props) = &p.properties {
            if let Some(s) = props.get("size") {
                size = parse_addr(s);
            } else if let Some(s) = props.get("region-size") {
                size = parse_addr(s);
            }
        }
        let size_hi = size >> ADDR_SHIFT;
        let size_lo = size & ADDR_MASK;
        dts.push_str(&format!(
            "{}    reg = <0x{:x} 0x{:x} 0x{:x} 0x{:x}>;\n",
            indent, addr_hi, addr_lo, size_hi, size_lo
        ));
    } else {
        dts.push_str(&format!("{}    reg = <{}>;\n", indent, addr));
    }

    if compat == "pl011" {
        dts.push_str(&format!("{}    chardev = <0x{:x}>;\n", indent, DEFAULT_CHARDEV_PHANDLE));
    }

    if compat == "arm_gic" {
        dts.push_str(&format!("{}    interrupt-controller;\n", indent));
        dts.push_str(&format!("{}    #interrupt-cells = <{}>;\n", indent, INTERRUPT_CELLS));
    }

    if p_type.starts_with("SPI") {
        dts.push_str(&format!("{}    #address-cells = <1>;\n", indent));
        dts.push_str(&format!("{}    #size-cells = <0>;\n", indent));
    }

    if let Some(irqs) = &p.interrupts {
        let mut cells = Vec::new();
        for irq in irqs {
            let num = match irq {
                serde_yaml::Value::Number(n) => n.as_u64().expect("Invalid data format"),
                serde_yaml::Value::String(s) => {
                    let parts: Vec<&str> = s.split('@').collect();
                    if parts.len() == IRQ_STRING_PARTS {
                        parts[1].parse::<u64>().expect("Invalid data format")
                    } else {
                        s.parse::<u64>().expect("Invalid data format")
                    }
                }
                _ => 0,
            };
            cells.push("0".to_string());
            cells.push(format!("{}", num + GIC_SPI_OFFSET));
            cells.push(format!("{}", DEFAULT_INTERRUPT_FLAG));
        }
        dts.push_str(&format!("{}    interrupts = <{}>;\n", indent, cells.join(" ")));

        if let Some(&g_ph) = phandles.get("gic") {
            dts.push_str(&format!("{}    interrupt-parent = <{}>;\n", indent, g_ph));
        }
    }

    let is_native = [
        "telemetry",
        "ieee802154",
        "zenoh-wifi",
        "wifi",
        "spi",
        "spi-echo",
        "canfd",
        "flexray",
        "lin",
        "clock",
        "ui",
        "actuator",
        "sensor",
        "s32k144-lpuart",
    ]
    .contains(&p_type);
    if is_native {
        dts.push_str(&format!("{}    transport = <{}>;\n", indent, HUB_PHANDLE));
        // phandle 2 is hub0
    }

    if let Some(props) = &p.properties {
        for (k, v) in props {
            let k_lower = k.to_lowercase();
            if k == "size"
                || k == "region-size"
                || k == "cpuType"
                || k == "isa"
                || k == "mmu-type"
                || k == "chardev"
                || k == "socket-path"
                || k == "base-addr"
                || k == "transport"
            {
                continue;
            }
            match v {
                serde_yaml::Value::String(s) => {
                    let k_qemu = if k_lower == "topic-prefix" {
                        "topic-prefix"
                    } else if k_lower == "macaddress" || k_lower == "macaddr" || k_lower == "mac" {
                        "macaddr"
                    } else {
                        k.as_str()
                    };

                    if k_qemu == "macaddr" {
                        dts.push_str(&format!("{}    {} = \"{}\";\n", indent, k_qemu, s));
                    } else if phandles.contains_key(s) {
                        dts.push_str(&format!(
                            "{}    {} = <{}>;\n",
                            indent,
                            k_qemu,
                            phandles.get(s).unwrap()
                        ));
                    } else {
                        dts.push_str(&format!("{}    {} = \"{}\";\n", indent, k_qemu, s));
                    }
                }
                serde_yaml::Value::Number(n) => {
                    if let Some(num) = n.as_u64() {
                        dts.push_str(&format!("{}    {} = <0x{:x}>;\n", indent, k, num));
                    }
                }
                serde_yaml::Value::Bool(b) if *b => {
                    dts.push_str(&format!("{}    {} = <1>;\n", indent, k));
                }
                _ => {}
            }
        }
    }

    if compat == "mmio-socket-bridge" {
        cli_args.push("-device".to_string());
        let region_size = if let Some(props) = &p.properties {
            if let Some(s) = props.get("region-size") {
                parse_addr(s)
            } else {
                DEFAULT_REGION_SIZE
            }
        } else {
            DEFAULT_REGION_SIZE
        };

        let sock = if let Some(props) = &p.properties {
            if let Some(serde_yaml::Value::String(s)) = props.get("socket-path") {
                s.clone()
            } else {
                "".to_string()
            }
        } else {
            "".to_string()
        };

        let svd_hash = if let Some(props) = &p.properties {
            if let Some(serde_yaml::Value::Number(n)) = props.get("svd-hash") {
                n.as_u64().expect("svd-hash property must be a valid u64")
            } else {
                0
            }
        } else {
            0
        };

        cli_args.push(format!(
            "mmio-socket-bridge,id={},base-addr={},region-size={},socket-path={},svd-hash={}",
            p.name, addr, region_size, sock, svd_hash
        ));
    }

    if let Some(children) = children_by_parent.get(&p.name) {
        let child_indent = format!("{}    ", indent);
        for child in children {
            emit_device(
                child,
                children_by_parent,
                phandles,
                &child_indent,
                dts,
                cli_args,
                endpoint,
            );
        }
    }

    dts.push_str(&format!("{}}};\n", indent));
}

pub mod validation;

pub use validation::validate_dtb;

pub fn parse_yaml(
    yaml_content: &str,
    endpoint: Option<&str>,
    node_id: u32,
) -> Result<(YamlPlatform, World)> {
    let world: World = serde_yaml::from_str(yaml_content)?;

    let mut dts = String::new();
    let mut cli_args = Vec::new();
    let mut has_clock = false;

    dts.push_str("/dts-v1/;\n/\n{\n");
    dts.push_str("    model = \"virtmcu-dynamic-machine\";\n");
    dts.push_str("    compatible = \"arm,generic-fdt\";\n");
    dts.push_str(&format!("    #address-cells = <{}>;\n", ADDR_CELLS));
    dts.push_str(&format!("    #size-cells = <{}>;\n\n", SIZE_CELLS));

    dts.push_str("    qemu_sysmem: qemu_sysmem {\n");
    dts.push_str("        compatible = \"qemu:system-memory\";\n");
    dts.push_str(&format!("        phandle = <{}>;\n", QEMU_SYSMEM_PHANDLE));
    dts.push_str("    };\n\n");

    let mut phandles = HashMap::new();
    phandles.insert("qemu_sysmem".to_string(), QEMU_SYSMEM_PHANDLE);
    phandles.insert("hub0".to_string(), HUB_PHANDLE as u64);
    let mut next_phandle = INITIAL_NEXT_PHANDLE;

    if let Some(m) = &world.machine {
        dts.push_str("    cpus {\n");
        dts.push_str(&format!("        #address-cells = <{}>;\n", CPU_ADDR_CELLS));
        dts.push_str(&format!("        #size-cells = <{}>;\n", CPU_SIZE_CELLS));
        for (i, cpu) in m.cpus.iter().enumerate() {
            let cpu_type = if cpu.cpu_type.starts_with("cortex") {
                format!("{}-arm-cpu", cpu.cpu_type)
            } else {
                cpu.cpu_type.clone()
            };
            dts.push_str(&format!("        {}@{} {{\n", cpu.name, i));
            dts.push_str("            device_type = \"cpu\";\n");
            dts.push_str(&format!("            compatible = \"{}\";\n", cpu_type));
            dts.push_str(&format!("            reg = <{}>;\n", i));
            dts.push_str(&format!("            memory = <{}>;\n", QEMU_SYSMEM_PHANDLE));
            dts.push_str("        };\n");
        }
        dts.push_str("    };\n\n");
    }

    let mut has_native = false;
    for p in &world.peripherals {
        let p_type = p.periph_type.as_deref().unwrap_or("Unknown");
        let is_native = [
            "telemetry",
            "ieee802154",
            "zenoh-wifi",
            "wifi",
            "spi",
            "spi-echo",
            "canfd",
            "flexray",
            "lin",
            "clock",
            "ui",
            "actuator",
            "sensor",
            "s32k144-lpuart",
        ]
        .contains(&p_type);
        if is_native {
            has_native = true;
            break;
        }
    }

    if has_native {
        // Inject hub0
        dts.push_str("    hub0 {\n");
        dts.push_str("        compatible = \"virtmcu-transport-hub\";\n");
        dts.push_str(&format!("        phandle = <{}>;\n", HUB_PHANDLE));
        if let Some(ep) = endpoint {
            dts.push_str(&format!("        router = \"{}\";\n", ep));
        }
        dts.push_str(&format!("        node = <{}>;\n", node_id));
        dts.push_str("    };\n\n");
    }

    // Pass 1: Assign phandles, bucket children, and apply SVD augmentation
    let mut children_by_parent: HashMap<String, Vec<Peripheral>> = HashMap::new();
    let mut augmented_peripherals = world.peripherals.clone();

    // SVD Augmentation
    for p in &mut augmented_peripherals {
        if let Some(props) = &mut p.properties {
            if let Some(serde_yaml::Value::String(svd_path)) = props.get("svd") {
                // Determine absolute path
                let svd_path_buf = if std::path::Path::new(svd_path).exists() {
                    std::path::PathBuf::from(svd_path)
                } else if let Ok(ws) = std::env::var("VIRTMCU_WORKSPACE") {
                    std::path::Path::new(&ws).join(svd_path)
                } else {
                    std::path::PathBuf::from(svd_path)
                };

                if svd_path_buf.exists() {
                    let xml = std::fs::read_to_string(&svd_path_buf).map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to read SVD file '{}': {}",
                            svd_path_buf.display(),
                            e
                        )
                    })?;

                    use std::hash::{Hash, Hasher};
                    let mut hasher = fnv::FnvHasher::default();
                    xml.hash(&mut hasher);
                    let svd_hash = hasher.finish();
                    props
                        .insert("svd-hash".to_string(), serde_yaml::Value::Number(svd_hash.into()));

                    let device = svd_parser::parse(&xml).map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to parse SVD file '{}': {:?}",
                            svd_path_buf.display(),
                            e
                        )
                    })?;
                    if let Some(svd_periph) = device.peripherals.first() {
                        // Update address if missing or "none"
                        let mut need_addr_update = p.address.is_none();
                        if let Some(serde_yaml::Value::String(s)) = &p.address {
                            if s == "none" {
                                need_addr_update = true;
                            }
                        }
                        if need_addr_update {
                            p.address = Some(serde_yaml::Value::String(format!(
                                "0x{:x}",
                                svd_periph.base_address
                            )));
                        } else {
                            // Lint check: if address is explicitly provided but doesn't match SVD baseAddress
                            if let Some(addr_val) = &p.address {
                                let current_addr = parse_addr(addr_val);
                                if current_addr != svd_periph.base_address {
                                    return Err(anyhow::anyhow!(
                                        "Lint Error: Peripheral '{}' has hardcoded address {:#x} which differs from SVD baseAddress {:#x}. Use 'address: none' to follow SOTA SSoT pattern.",
                                        p.name, current_addr, svd_periph.base_address
                                    ));
                                }
                            }
                        }

                        // Update size if missing
                        if !props.contains_key("size") && !props.contains_key("region-size") {
                            if let Some(ab) = svd_periph.address_block.as_ref() {
                                if !ab.is_empty() {
                                    props.insert(
                                        "size".to_string(),
                                        serde_yaml::Value::String(format!("0x{:x}", ab[0].size)),
                                    );
                                }
                            }
                        }
                    }
                } else {
                    return Err(anyhow::anyhow!(
                        "SVD file not found: '{}' (resolved to '{}').",
                        svd_path,
                        svd_path_buf.display()
                    ));
                }
                props.remove("svd");
            }
        }
    }

    for p in &augmented_peripherals {
        if p.periph_type.as_deref() == Some("clock") {
            has_clock = true;
        }
        phandles.insert(p.name.clone(), next_phandle);
        next_phandle += 1;
        if let Some(parent) = &p.parent {
            children_by_parent.entry(parent.clone()).or_default().push(p.clone());
        }
    }

    // Pass 2: Emit root peripherals
    for p in &augmented_peripherals {
        if p.parent.is_none() {
            emit_device(
                p,
                &children_by_parent,
                &phandles,
                "    ",
                &mut dts,
                &mut cli_args,
                endpoint,
            );
        }
    }

    dts.push_str("};\n");

    Ok((YamlPlatform { dts_content: dts, cli_args, has_clock }, world))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_yaml_basic() {
        let yaml = format!(
            r#"
machine:
  cpus:
    - name: cpu0
      type: cortex-m4
peripherals:
  - name: uart0
    type: s32k144-lpuart
    address: 0x{:x}
    interrupts:
      - {}
"#,
            EXAMPLE_UART_ADDR, EXAMPLE_UART_IRQ
        );
        let result = parse_yaml(&yaml, Some(EXAMPLE_ROUTER_ENDPOINT), EXAMPLE_NODE_ID);
        assert!(result.is_ok());
        let (platform, _world) = result.unwrap();
        assert!(platform.dts_content.contains("cpu0"));
        assert!(platform.dts_content.contains("uart0"));
        assert!(platform.dts_content.contains("compatible = \"s32k144-lpuart\""));
        assert!(platform.dts_content.contains(&format!(
            "reg = <0x0 0x{:x} 0x0 0x{:x}>",
            EXAMPLE_UART_ADDR, DEFAULT_REGION_SIZE
        )));
        assert!(platform.dts_content.contains(&format!(
            "interrupts = <0 {} {}>",
            EXPECTED_GIC_IRQ, DEFAULT_INTERRUPT_FLAG
        )));
    }

    #[test]
    fn test_parse_yaml_with_clock() {
        let yaml = r#"
peripherals:
  - name: clock0
    type: clock
    address: none
"#;
        let result = parse_yaml(yaml, None, 0);
        assert!(result.is_ok());
        let (platform, _) = result.unwrap();
        assert!(platform.has_clock);
    }

    #[test]
    fn test_parse_yaml_mac_parsing() {
        let yaml = r#"
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals:
  - name: ram
    type: Memory.MappedMemory
    address: 0x40000000
    properties:
      size: 0x1000000
  - name: test_dev
    type: test-rust-device
    address: sysbus
    properties:
      MACAddress: "00:11:22:33:44:55"
"#;
        let result = parse_yaml(yaml, None, 0);
        assert!(result.is_ok());
        let (platform, _) = result.unwrap();
        println!("Generated DTS:\n{}", platform.dts_content);
    }

    #[test]
    fn test_mismatched_svd_address() {
        let yaml_path =
            std::path::Path::new("../../../tests/fixtures/guest_apps/mismatched_svd/board.yaml");
        let yaml = std::fs::read_to_string(yaml_path).expect("Failed to read fixture");
        let result = parse_yaml(&yaml, None, 0);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Lint Error: Peripheral 'actuator0' has hardcoded address 0xb000000 which differs from SVD baseAddress 0xa000000. Use 'address: none' to follow SOTA SSoT pattern."));
    }
}

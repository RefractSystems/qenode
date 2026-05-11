use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

pub struct YamlPlatform {
    pub dts_content: String,
    pub cli_args: Vec<String>,
    pub has_clock: bool,
}

fn parse_addr(val: &serde_yaml::Value) -> u64 {
    match val {
        serde_yaml::Value::Number(n) => n.as_u64().unwrap_or(0),
        serde_yaml::Value::String(s) => {
            let s = s.trim();
            if let Some(stripped) = s.strip_prefix("0x") {
                u64::from_str_radix(stripped, 16).unwrap_or(0)
            } else {
                s.parse::<u64>().unwrap_or(0)
            }
        }
        _ => 0,
    }
}

fn emit_device(
    p: &Peripheral,
    children_by_parent: &HashMap<String, Vec<&Peripheral>>,
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
            if let Some(serde_yaml::Value::String(n)) = props.get("node") { n.as_str() } else { "0" }
        } else { "0" };

        let topic = if let Some(props) = &p.properties {
            if let Some(serde_yaml::Value::String(t)) = props.get("topic") { format!(",topic={}", t) } else { "".to_string() }
        } else { "".to_string() };

        let router_arg = if let Some(ep) = endpoint { format!(",router={}", ep) } else { "".to_string() };

        cli_args.push("-chardev".to_string());
        cli_args.push(format!("virtmcu,id=chr_{},node={},transport=zenoh{}{}", p.name, node, router_arg, topic));
        return; // chardevs don't emit DT nodes
    }

    if p_type == "Memory.MappedMemory" {
        let size = if let Some(props) = &p.properties {
            if let Some(s) = props.get("size") { parse_addr(s) } else { 0x1000 }
        } else { 0x1000 };

        dts.push_str(&format!("{}memory@{:x} {{\n", indent, addr));
        dts.push_str(&format!("{}    compatible = \"qemu-memory-region\";\n", indent));
        dts.push_str(&format!("{}    qemu,ram = <0x01>;\n", indent));
        dts.push_str(&format!("{}    container = <1>;\n", indent));

        let addr_hi = addr >> 32;
        let addr_lo = addr & 0xFFFFFFFF;
        let size_hi = size >> 32;
        let size_lo = size & 0xFFFFFFFF;

        dts.push_str(&format!("{}    reg = <0x{:x} 0x{:x} 0x{:x} 0x{:x}>;\n", indent, addr_hi, addr_lo, size_hi, size_lo));
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
    dts.push_str(&format!("{}    phandle = <{}>;\n", indent, phandles.get(&p.name).unwrap_or(&0)));

    let addr_hi = addr >> 32;
    let addr_lo = addr & 0xFFFFFFFF;

    if p.parent.is_none() {
        let mut size = 0x1000;
        if let Some(props) = &p.properties {
            if let Some(s) = props.get("size") {
                size = parse_addr(s);
            } else if let Some(s) = props.get("region-size") {
                size = parse_addr(s);
            }
        }
        let size_hi = size >> 32;
        let size_lo = size & 0xFFFFFFFF;
        dts.push_str(&format!("{}    reg = <0x{:x} 0x{:x} 0x{:x} 0x{:x}>;\n", indent, addr_hi, addr_lo, size_hi, size_lo));
    } else {
        dts.push_str(&format!("{}    reg = <{}>;\n", indent, addr));
    }

    if compat == "pl011" {
        dts.push_str(&format!("{}    chardev = <0x00>;\n", indent));
    }

    if compat == "arm_gic" {
        dts.push_str(&format!("{}    interrupt-controller;\n", indent));
        dts.push_str(&format!("{}    #interrupt-cells = <3>;\n", indent));
    }

    if p_type.starts_with("SPI") {
        dts.push_str(&format!("{}    #address-cells = <1>;\n", indent));
        dts.push_str(&format!("{}    #size-cells = <0>;\n", indent));
    }

    if let Some(irqs) = &p.interrupts {
        let mut cells = Vec::new();
        for irq in irqs {
            let num = match irq {
                serde_yaml::Value::Number(n) => n.as_u64().unwrap_or(0),
                serde_yaml::Value::String(s) => {
                    let parts: Vec<&str> = s.split('@').collect();
                    if parts.len() == 2 {
                        parts[1].parse::<u64>().unwrap_or(0)
                    } else {
                        s.parse::<u64>().unwrap_or(0)
                    }
                }
                _ => 0,
            };
            cells.push("0".to_string());
            cells.push(format!("{}", num + 32));
            cells.push("4".to_string());
        }
        dts.push_str(&format!("{}    interrupts = <{}>;\n", indent, cells.join(" ")));

        if let Some(&g_ph) = phandles.get("gic") {
            dts.push_str(&format!("{}    interrupt-parent = <{}>;\n", indent, g_ph));
        }
    }

    let is_native = ["telemetry", "ieee802154", "zenoh-wifi", "wifi", "spi", "spi-echo", "canfd", "flexray", "lin", "clock", "ui", "actuator", "s32k144-lpuart"].contains(&p_type);
    if is_native {
        dts.push_str(&format!("{}    transport = <2>;\n", indent)); // phandle 2 is hub0
    }

    if let Some(props) = &p.properties {
        for (k, v) in props {
            let k_lower = k.to_lowercase();
            if k == "size" || k == "region-size" || k == "cpuType" || k == "isa" || k == "mmu-type" || k == "chardev" || k == "socket-path" || k == "base-addr" || k == "transport" {
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
                        dts.push_str(&format!("{}    {} = <{}>;\n", indent, k_qemu, phandles.get(s).unwrap()));
                    } else {
                        dts.push_str(&format!("{}    {} = \"{}\";\n", indent, k_qemu, s));
                    }
                }
                serde_yaml::Value::Number(n) => {
                    if let Some(num) = n.as_u64() {
                        dts.push_str(&format!("{}    {} = <0x{:x}>;\n", indent, k, num));
                    }
                }
                serde_yaml::Value::Bool(b) => {
                    if *b {
                        dts.push_str(&format!("{}    {};\n", indent, k));
                    }
                }
                _ => {}
            }
        }
    }

    if compat == "mmio-socket-bridge" {
        cli_args.push("-device".to_string());
        let region_size = if let Some(props) = &p.properties {
            if let Some(s) = props.get("region-size") { parse_addr(s) } else { 0x1000 }
        } else { 0x1000 };

        let sock = if let Some(props) = &p.properties {
            if let Some(serde_yaml::Value::String(s)) = props.get("socket-path") { s.clone() } else { "".to_string() }
        } else { "".to_string() };

        cli_args.push(format!("mmio-socket-bridge,id={},base-addr={},region-size={},socket-path={}", p.name, addr, region_size, sock));
    }

    if let Some(children) = children_by_parent.get(&p.name) {
        let child_indent = format!("{}    ", indent);
        for child in children {
            emit_device(child, children_by_parent, phandles, &child_indent, dts, cli_args, endpoint);
        }
    }

    dts.push_str(&format!("{}}};\n", indent));
}

pub fn parse_yaml(yaml_content: &str, endpoint: Option<&str>, node_id: u32) -> Result<YamlPlatform> {
    let world: World = serde_yaml::from_str(yaml_content)?;

    let mut dts = String::new();
    let mut cli_args = Vec::new();
    let mut has_clock = false;

    dts.push_str("/dts-v1/;\n/\n{\n");
    dts.push_str("    model = \"virtmcu-dynamic-machine\";\n");
    dts.push_str("    compatible = \"arm,generic-fdt\";\n");
    dts.push_str("    #address-cells = <2>;\n");
    dts.push_str("    #size-cells = <2>;\n\n");

    dts.push_str("    qemu_sysmem: qemu_sysmem {\n");
    dts.push_str("        compatible = \"qemu:system-memory\";\n");
    dts.push_str("        phandle = <1>;\n");
    dts.push_str("    };\n\n");

    let mut phandles = HashMap::new();
    phandles.insert("qemu_sysmem".to_string(), 1);
    phandles.insert("hub0".to_string(), 2);
    let mut next_phandle = 3;

    if let Some(m) = &world.machine {
        dts.push_str("    cpus {\n");
        dts.push_str("        #address-cells = <1>;\n");
        dts.push_str("        #size-cells = <0>;\n");
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
            dts.push_str("            memory = <1>;\n");
            dts.push_str("        };\n");
        }
        dts.push_str("    };\n\n");
    }

    // Inject hub0
    dts.push_str("    hub0 {\n");
    dts.push_str("        compatible = \"virtmcu-transport-hub\";\n");
    dts.push_str("        phandle = <2>;\n");
    if let Some(ep) = endpoint {
        dts.push_str(&format!("        router = \"{}\";\n", ep));
    }
    dts.push_str(&format!("        node = <{}>;\n", node_id));
    dts.push_str("    };\n\n");

    // Pass 1: Assign phandles and bucket children
    let mut children_by_parent: HashMap<String, Vec<&Peripheral>> = HashMap::new();
    for p in &world.peripherals {
        if p.periph_type.as_deref() == Some("clock") {
            has_clock = true;
        }
        phandles.insert(p.name.clone(), next_phandle);
        next_phandle += 1;
        if let Some(parent) = &p.parent {
            children_by_parent.entry(parent.clone()).or_default().push(p);
        }
    }

    // Pass 2: Emit root peripherals
    for p in &world.peripherals {
        if p.parent.is_none() {
            emit_device(p, &children_by_parent, &phandles, "    ", &mut dts, &mut cli_args, endpoint);
        }
    }

    dts.push_str("};\n");

    Ok(YamlPlatform {
        dts_content: dts,
        cli_args,
        has_clock,
    })
}

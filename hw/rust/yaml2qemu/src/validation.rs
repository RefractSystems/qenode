use crate::World;
use anyhow::{anyhow, Result};
use fdt::Fdt;
use std::path::Path;
use tracing::{error, info};

const REG_PROP_LEN_TWO_CELLS: usize = 16; // 4 u32 cells = 16 bytes
const SIZE_HI_OFFSET: usize = 8;
const SIZE_HI_END: usize = 12;
const SIZE_LO_OFFSET: usize = 12;
const SIZE_LO_END: usize = 16;
const ADDR_SHIFT: u32 = 32;
const HEX_RADIX: u32 = 16;

pub fn validate_dtb(dtb_path: &Path, world: &World) -> Result<()> {
    let dtb_data = std::fs::read(dtb_path)?;
    let fdt = match Fdt::new(&dtb_data) {
        Ok(f) => f,
        Err(e) => return Err(anyhow!("Failed to parse DTB: {:?}", e)),
    };

    let mut missing = Vec::new();

    if let Some(m) = &world.machine {
        if let Some(cpus_node) = fdt.find_node("/cpus") {
            for cpu in &m.cpus {
                let cpu_name = format!("{}@", cpu.name);
                let mut found = false;
                for child in cpus_node.children() {
                    if child.name.starts_with(&cpu_name) {
                        found = true;
                        if child.property("memory").is_none() {
                            error!("ERROR: CPU node '{}' is missing 'memory' property!", cpu.name);
                            missing.push(format!("{} (missing memory binding)", cpu.name));
                        }
                        break;
                    }
                }
                if !found {
                    missing.push(cpu.name.clone());
                }
            }
        } else {
            error!("ERROR: No 'cpus' node found in DTB!");
            missing.push("cpus".to_string());
        }
    }

    for dev in &world.peripherals {
        if dev.periph_type.as_deref() == Some("chardev") {
            continue;
        }

        let prefix = if dev.periph_type.as_deref() == Some("Memory.MappedMemory") {
            if dev.name.contains('@') {
                format!("memory@{}", dev.name.split('@').nth(1).unwrap())
            } else if let Some(serde_yaml::Value::String(s)) = &dev.address {
                let addr_hex = s.trim_start_matches("0x");
                format!("memory@{}", addr_hex)
            } else if let Some(serde_yaml::Value::Number(n)) = &dev.address {
                format!(
                    "memory@{:x}",
                    n.as_u64().expect("Memory address should be parseable as u64")
                )
            } else {
                "memory".to_string()
            }
        } else {
            dev.name.clone()
        };

        let dev_node = find_node_recursive(&fdt, &prefix);

        if let Some(node) = dev_node {
            if dev.periph_type.as_deref() == Some("Memory.MappedMemory") {
                if let Some(props) = &dev.properties {
                    if let Some(size_val) = props.get("size") {
                        if let Some(reg) = node.property("reg") {
                            let data = reg.value;
                            if data.len() >= REG_PROP_LEN_TWO_CELLS {
                                let size_hi = u32::from_be_bytes(
                                    data[SIZE_HI_OFFSET..SIZE_HI_END].try_into().unwrap(),
                                ) as u64;
                                let size_lo = u32::from_be_bytes(
                                    data[SIZE_LO_OFFSET..SIZE_LO_END].try_into().unwrap(),
                                ) as u64;
                                let actual_size = (size_hi << ADDR_SHIFT) | size_lo;

                                let expected_size = match size_val {
                                    serde_yaml::Value::String(s) => {
                                        let s = s.trim();
                                        if let Some(stripped) = s.strip_prefix("0x") {
                                            u64::from_str_radix(stripped, HEX_RADIX)
                                                .expect("Invalid hex string for memory size")
                                        } else {
                                            s.parse::<u64>()
                                                .expect("Invalid decimal string for memory size")
                                        }
                                    }
                                    serde_yaml::Value::Number(n) => {
                                        n.as_u64().expect("Memory size must be parseable as u64")
                                    }
                                    _ => unreachable!("Memory size must be a number or string"),
                                };

                                if actual_size != expected_size {
                                    error!("ERROR: Memory node '{}' size mismatch! Expected {:#x}, found {:#x}", dev.name, expected_size, actual_size);
                                    missing.push(format!("{} (size mismatch)", dev.name));
                                }
                            } else {
                                error!(
                                    "Memory node '{}' has unexpected reg property length: {}",
                                    dev.name,
                                    data.len()
                                );
                            }
                        } else {
                            error!("ERROR: Memory node '{}' is missing 'reg' property!", dev.name);
                            missing.push(format!("{} (missing reg)", dev.name));
                        }
                    }
                }
            }
        } else {
            missing.push(dev.name.clone());
        }
    }

    if !missing.is_empty() {
        error!(
            "ERROR: The following peripherals from YAML are missing in the generated DTB: {}",
            missing.join(", ")
        );
        error!("This usually means the device type is unknown to FdtEmitter or the address mapping failed.");
        error!("FAILED: DTB validation failed.");
        return Err(anyhow!("DTB validation failed"));
    }

    info!("✓ Validation successful.");
    Ok(())
}

fn find_node_recursive<'a, 'b>(
    fdt: &'a Fdt<'b>,
    prefix: &str,
) -> Option<fdt::node::FdtNode<'b, 'a>> {
    let prefix_at = format!("{}@", prefix);
    fdt.all_nodes().find(|node| node.name == prefix || node.name.starts_with(&prefix_at))
}

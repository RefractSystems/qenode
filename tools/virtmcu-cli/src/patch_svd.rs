use anyhow::Result;
use regex::Regex;
use serde_yaml::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub async fn run_platform_patch_svd(
    input: PathBuf,
    patch: PathBuf,
    output: Option<PathBuf>,
) -> Result<()> {
    let xml = std::fs::read_to_string(&input)?;
    let patch_content = std::fs::read_to_string(&patch)?;

    let patch_data: Value = serde_yaml::from_str(&patch_content)?;

    let patched_xml = apply_svd_patch(&xml, &patch_data)?;

    if let Some(out_path) = output {
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, patched_xml)?;
        tracing::info!("✓ Patched SVD saved to {}", out_path.display());
    } else {
        println!("{}", patched_xml);
    }

    Ok(())
}

fn apply_svd_patch(xml: &str, patch: &Value) -> Result<String> {
    // Parse the YAML patch into a more accessible structure
    // peripherals -> ACTUATOR -> registers -> GO -> resetValue: 0x1
    let mut periph_patches: HashMap<String, HashMap<String, HashMap<String, String>>> =
        HashMap::new();

    if let Some(peripherals) = patch.get("peripherals").and_then(|v| v.as_mapping()) {
        for (p_key, p_val) in peripherals {
            let p_name = p_key.as_str().unwrap_or("").to_string();
            let mut reg_patches = HashMap::new();

            if let Some(registers) = p_val.get("registers").and_then(|v| v.as_mapping()) {
                for (r_key, r_val) in registers {
                    let r_name = r_key.as_str().unwrap_or("").to_string();
                    let mut props = HashMap::new();

                    if let Some(mapping) = r_val.as_mapping() {
                        for (k, v) in mapping {
                            let k_str = k.as_str().unwrap_or("").to_string();
                            let v_str = if let Some(s) = v.as_str() {
                                s.to_string()
                            } else if let Some(i) = v.as_u64() {
                                format!("{:#x}", i)
                            } else if let Some(i) = v.as_i64() {
                                i.to_string()
                            } else {
                                v.as_f64()
                                    .expect("Invalid numeric format in YAML patch")
                                    .to_string()
                            };
                            props.insert(k_str, v_str);
                        }
                    }
                    reg_patches.insert(r_name, props);
                }
            }
            periph_patches.insert(p_name, reg_patches);
        }
    }

    // Basic state machine to patch the SVD
    let mut output = String::new();

    let mut current_periph: Option<String> = None;
    let mut current_reg: Option<String> = None;
    let mut indent = String::new();

    let re_periph_start = Regex::new(r"^\s*<peripheral>\s*$").unwrap();
    let re_periph_end = Regex::new(r"^\s*</peripheral>\s*$").unwrap();
    let re_reg_start = Regex::new(r"^(\s*)<register>\s*$").unwrap();
    let re_reg_end = Regex::new(r"^\s*</register>\s*$").unwrap();
    let re_name = Regex::new(r"^\s*<name>(.*?)</name>\s*$").unwrap();

    let mut in_patched_reg = false;
    let mut patched_props: HashMap<String, String> = HashMap::new();
    let mut seen_props = std::collections::HashSet::new();

    for line in xml.lines() {
        if re_periph_start.is_match(line) {
            current_periph = Some("".to_string());
        } else if re_periph_end.is_match(line) {
            current_periph = None;
        } else if let Some(caps) = re_reg_start.captures(line) {
            current_reg = Some("".to_string());
            indent = caps.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            in_patched_reg = false;
            seen_props.clear();
        } else if re_reg_end.is_match(line) {
            // Inject any properties that were patched but not seen in the original XML
            if in_patched_reg {
                for (prop, val) in &patched_props {
                    if !seen_props.contains(prop) {
                        output.push_str(&format!("  {}<{}>{}</{}>\n", indent, prop, val, prop));
                    }
                }
            }
            current_reg = None;
            in_patched_reg = false;
        } else if let Some(caps) = re_name.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            if current_reg.is_some() {
                current_reg = Some(name.clone());

                // Check if this register needs patching
                if let Some(p_name) = &current_periph {
                    if let Some(r_patches) = periph_patches.get(p_name) {
                        if let Some(props) = r_patches.get(&name) {
                            in_patched_reg = true;
                            patched_props = props.clone();
                        }
                    }
                }
            } else if current_periph.is_some() {
                current_periph = Some(name);
            }
        }

        let mut modified_line = line.to_string();

        // If we are in a register that needs patching, check if this line contains a property we want to replace
        if in_patched_reg {
            for (prop, val) in &patched_props {
                let prop_re = Regex::new(&format!(r"^(\s*)<{}>.*?</{}>\s*$", prop, prop)).unwrap();
                if let Some(caps) = prop_re.captures(line) {
                    let prop_indent = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    modified_line = format!("{}<{}>{}</{}>", prop_indent, prop, val, prop);
                    seen_props.insert(prop.clone());
                }
            }
        }

        output.push_str(&modified_line);
        output.push('\n');
    }

    Ok(output)
}

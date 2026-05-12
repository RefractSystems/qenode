use anyhow::Result;
use ignore::WalkBuilder;
use std::path::Path;
use tracing::{error, info};

use crate::lints::static_state::Lint;

pub struct RustSafeSerializationLint;

impl Lint for RustSafeSerializationLint {
    fn name(&self) -> &'static str {
        "rust_safe_serialization"
    }

    fn check(&self, target_dir: &Path) -> Result<bool> {
        let mut passed = true;
        let hw_rust_dir = target_dir.join("hw/rust");

        if !hw_rust_dir.exists() {
            return Ok(true);
        }

        let walker = WalkBuilder::new(&hw_rust_dir)
            .add_custom_ignore_filename(".geminiignore")
            .build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }

            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if file_name.ends_with("_generated.rs") {
                continue;
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.contains("ptr::copy_nonoverlapping") {
                    if is_suppressed(&lines, i, "copy") {
                        continue;
                    }
                    passed = false;
                    error!(
                        "{}:{}: Banned ptr::copy_nonoverlapping found (Safe Endianness Serialization Mandate).\n  Fix: Use .to_le_bytes() / .from_le_bytes() instead.",
                        path.display(), i + 1
                    );
                }
            }
        }

        if passed {
            info!("✓ Safe Endianness Serialization Rust lint passed.");
        }

        Ok(passed)
    }
}

fn is_suppressed(lines: &[&str], line_idx: usize, rule: &str) -> bool {
    let suppression_pattern = format!("// virtmcu-allow: {}", rule);

    if line_idx < lines.len() && lines[line_idx].contains(&suppression_pattern) {
        return true;
    }

    if line_idx > 0 && lines[line_idx - 1].contains(&suppression_pattern) {
        return true;
    }

    false
}

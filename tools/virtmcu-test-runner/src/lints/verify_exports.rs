use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Command;
use tracing::{error, info};

use crate::lints::static_state::Lint;

pub struct ExportLint;

impl Lint for ExportLint {
    fn name(&self) -> &'static str {
        "verify_exports"
    }

    fn check(&self, target_dir: &Path) -> Result<bool> {
        let mut passed = true;

        // Mandated symbols that the main QEMU executable MUST export dynamically to plugins
        let qemu_required_exports = vec![
            "virtmcu_cpu_set_tcg_hook",
            "virtmcu_cpu_set_halt_hook",
            "virtmcu_set_irq_hook",
            "virtmcu_kick_first_cpu_for_quantum",
            "virtmcu_vcpu_should_yield",
            "virtmcu_is_bql_locked",
            "virtmcu_safe_bql_unlock",
            "virtmcu_safe_bql_lock",
            "virtmcu_safe_bql_force_unlock",
            "virtmcu_safe_bql_force_lock",
        ];

        let qemu_bin = target_dir.join("third_party/qemu/build-virtmcu/qemu-system-arm");
        if qemu_bin.exists() {
            if !check_symbols(&qemu_bin, &qemu_required_exports, true)? {
                passed = false;
            }

            // Check plugins
            let required_plugins = vec![("hw-virtmcu-clock.so", vec!["clock_cpu_halt_cb"])];

            let build_dir = qemu_bin.parent().unwrap();
            for (so_name, symbols) in required_plugins {
                // Search for so_name in build_dir
                let mut found_plugin = None;
                for entry in walkdir::WalkDir::new(build_dir)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_name().to_string_lossy() == so_name {
                        found_plugin = Some(entry.path().to_path_buf());
                        break;
                    }
                }

                if let Some(so_path) = found_plugin {
                    if !check_symbols(&so_path, &symbols, false)? {
                        passed = false;
                    }
                }
            }
        } else {
            info!(
                "QEMU binary not found at {}, skipping export check.",
                qemu_bin.display()
            );
        }

        if passed {
            info!("✓ Export symbols check passed.");
        }

        Ok(passed)
    }
}

fn check_symbols(path: &Path, required: &[&str], is_executable: bool) -> Result<bool> {
    let target_type = if is_executable {
        "executable"
    } else {
        "plugin"
    };
    info!(
        "Checking {} {} for required FFI symbols...",
        target_type,
        path.display()
    );

    let nm_tool = if Command::new("llvm-nm").arg("--version").output().is_ok() {
        "llvm-nm"
    } else if Command::new("nm").arg("--version").output().is_ok() {
        "nm"
    } else {
        return Err(anyhow!("Neither 'llvm-nm' nor 'nm' found in PATH"));
    };

    let output = Command::new(nm_tool).arg("-D").arg(path).output()?;

    if !output.status.success() {
        error!("{} -D failed for {}", nm_tool, path.display());
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut exported_symbols = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        // nm -D output looks like:
        // 0000000000001234 T my_symbol
        // or for undefined:
        //                  U other_symbol

        let has_type = line.contains(" T ")
            || line.contains(" B ")
            || line.contains(" D ")
            || line.contains(" W ");
        if has_type {
            if let Some(symbol) = parts.last() {
                exported_symbols.push(*symbol);
            }
        }
    }

    let mut missing = Vec::new();
    for &s in required {
        if !exported_symbols.contains(&s) {
            missing.push(s);
        }
    }

    if !missing.is_empty() {
        error!(
            "{} is missing mandatory symbols: {:?}",
            path.display(),
            missing
        );
        if !is_executable {
            error!("  Ensure these are marked with #[no_mangle] extern \"C\" in Rust.");
        } else {
            error!("  Ensure these are marked with __attribute__((visibility(\"default\"))) in QEMU C code.");
        }
        return Ok(false);
    }

    info!(
        "✅ {}: All symbols found.",
        path.file_name().unwrap().to_string_lossy()
    );
    Ok(true)
}

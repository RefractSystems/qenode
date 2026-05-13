use anyhow::{anyhow, Result};
use std::env;
use std::path::PathBuf;

pub struct QemuLauncher {
    pub workspace_root: PathBuf,
}

impl QemuLauncher {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn build_dir_name(&self) -> String {
        let asan = env::var("VIRTMCU_USE_ASAN").unwrap_or_else(|_| "0".to_string()) == "1";
        let tsan = env::var("VIRTMCU_USE_TSAN").unwrap_or_else(|_| "0".to_string()) == "1";
        let mut name = "build-virtmcu".to_string();
        if asan {
            name.push_str("-asan");
        }
        if tsan {
            name.push_str("-tsan");
        }
        name
    }

    pub fn resolve_qemu_bin(&self, arch: &str) -> Result<PathBuf> {
        let build_dir = self.build_dir_name();
        let qemu_arch_name = match arch {
            "riscv" | "riscv64" => "riscv64",
            "riscv32" => "riscv32",
            _ => "arm",
        };

        if let Ok(bin) = env::var("QEMU_BIN") {
            if !bin.is_empty() {
                return Ok(PathBuf::from(bin));
            }
        }

        let use_prebuilt =
            env::var("VIRTMCU_USE_PREBUILT_QEMU").unwrap_or_else(|_| "0".to_string()) == "1";

        let mut possible_paths = Vec::new();

        if use_prebuilt {
            possible_paths.push(PathBuf::from(format!(
                "/build/qemu/{}/install/bin/qemu-system-{}",
                build_dir, qemu_arch_name
            )));
            possible_paths.push(PathBuf::from(format!(
                "/build/qemu/build-virtmcu/install/bin/qemu-system-{}",
                qemu_arch_name
            )));
        } else {
            // Local workspace paths
            let qemu_root = self.workspace_root.join("third_party/qemu");

            // Try build_dir-specific paths
            possible_paths
                .push(qemu_root.join(format!("{}/qemu-system-{}", build_dir, qemu_arch_name)));
            possible_paths.push(qemu_root.join(format!(
                "{}/install/bin/qemu-system-{}",
                build_dir, qemu_arch_name
            )));

            // Fallback to default build-virtmcu if build_dir is different (e.g. asan) but not found
            if build_dir != "build-virtmcu" {
                possible_paths
                    .push(qemu_root.join(format!("build-virtmcu/qemu-system-{}", qemu_arch_name)));
                possible_paths.push(qemu_root.join(format!(
                    "build-virtmcu/install/bin/qemu-system-{}",
                    qemu_arch_name
                )));
            }

            // Also check for qemu-bundle paths seen in some environments
            possible_paths.push(qemu_root.join(format!(
                "{}/qemu-bundle/workspace/third_party/qemu/{}/install/bin/qemu-system-{}",
                build_dir, build_dir, qemu_arch_name
            )));
        }

        for path in &possible_paths {
            if path.exists() && path.is_file() {
                return Ok(path.clone());
            }
        }

        Err(anyhow!(
            "QEMU binary for {} not found. Checked: {:?}",
            arch,
            possible_paths
        ))
    }

    pub fn get_module_dir(&self) -> Option<PathBuf> {
        let build_dir = self.build_dir_name();
        let use_prebuilt =
            env::var("VIRTMCU_USE_PREBUILT_QEMU").unwrap_or_else(|_| "0".to_string()) == "1";

        let qemu_base = if use_prebuilt {
            PathBuf::from(format!("/build/qemu/{}", build_dir))
        } else {
            self.workspace_root
                .join("third_party/qemu")
                .join(&build_dir)
        };

        // If specific build_dir (e.g. asan) doesn't exist for prebuilt, fallback to default
        let qemu_base = if use_prebuilt && !qemu_base.exists() {
            PathBuf::from("/build/qemu/build-virtmcu")
        } else {
            qemu_base
        };

        let arch = std::env::consts::ARCH;
        let mut candidate_subdirs = Vec::new();

        if arch == "aarch64" {
            candidate_subdirs.push("install/lib/aarch64-linux-gnu/qemu");
            candidate_subdirs.push("install/lib/x86_64-linux-gnu/qemu");
        } else {
            candidate_subdirs.push("install/lib/x86_64-linux-gnu/qemu");
            candidate_subdirs.push("install/lib/aarch64-linux-gnu/qemu");
        }
        candidate_subdirs.push("install/lib/qemu");
        candidate_subdirs.push("install/lib64/qemu");
        candidate_subdirs.push("lib/qemu");
        candidate_subdirs.push("lib64/qemu");

        for subdir in candidate_subdirs {
            let path = qemu_base.join(subdir);
            if path.exists() && path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        let file_name = entry.file_name();
                        if file_name.to_string_lossy().starts_with("hw-virtmcu-")
                            && file_name.to_string_lossy().contains(".so")
                        {
                            return Some(path);
                        }
                    }
                }
            }
        }
        None
    }

    pub fn get_ld_library_path(&self) -> String {
        let mut target_dirs = vec![
            self.workspace_root.join("target"),
            self.workspace_root.join("target-asan"),
            self.workspace_root.join("target-tsan"),
        ];
        if let Ok(target_dir_env) = env::var("CARGO_TARGET_DIR") {
            target_dirs.push(PathBuf::from(target_dir_env));
        }

        let mut paths = Vec::new();
        paths.push(
            self.workspace_root
                .join("third_party/zenoh-c/lib")
                .display()
                .to_string(),
        );

        let host_triple = match std::env::consts::ARCH {
            "aarch64" => "aarch64-unknown-linux-gnu",
            "x86_64" => "x86_64-unknown-linux-gnu",
            _ => "unknown",
        };

        for base in &target_dirs {
            // Prioritize the current host's target directory
            let host_target = base.join(host_triple);
            if host_target.exists() {
                paths.push(host_target.join("debug").display().to_string());
                paths.push(host_target.join("release").display().to_string());
                paths.push(host_target.join("debug/deps").display().to_string());
            }

            paths.push(base.join("debug").display().to_string());
            paths.push(base.join("release").display().to_string());
            paths.push(base.join("debug/deps").display().to_string());

            // Then check other triple-prefixed subdirectories
            if let Ok(entries) = std::fs::read_dir(base) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let path = entry.path();
                        let dir_name = path.file_name().unwrap_or_default().to_string_lossy();
                        if dir_name == host_triple {
                            continue; // Already added
                        }
                        if dir_name.contains("-unknown-linux-gnu") {
                            paths.push(path.join("debug").display().to_string());
                            paths.push(path.join("release").display().to_string());
                            paths.push(path.join("debug/deps").display().to_string());
                        }
                    }
                }
            }
        }

        let existing = env::var("LD_LIBRARY_PATH").unwrap_or_default();
        if !existing.is_empty() {
            paths.push(existing);
        }
        paths.join(":")
    }
}

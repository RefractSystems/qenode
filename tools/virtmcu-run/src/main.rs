use anyhow::{anyhow, Result};
use std::env;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::NamedTempFile;

fn find_workspace_root() -> Result<PathBuf> {
    let mut current_dir = env::current_dir()?;
    loop {
        if current_dir.join("VERSION").exists() || current_dir.join(".git").exists() {
            return Ok(current_dir);
        }
        if !current_dir.pop() {
            break;
        }
    }
    Err(anyhow!(
        "Could not find workspace root (looking for VERSION or .git)"
    ))
}

fn build_dir_name() -> String {
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

fn resolve_qemu_bin(arch: &str, workspace_root: &Path, build_dir: &str) -> Result<PathBuf> {
    let qemu_arch_name = match arch {
        "riscv" | "riscv64" => "riscv64",
        "riscv32" => "riscv32",
        _ => "arm", // default to arm
    };

    // Global override
    if let Ok(bin) = env::var("QEMU_BIN") {
        if !bin.is_empty() {
            println!("==> Using global QEMU_BIN: {}", bin);
            return Ok(PathBuf::from(bin));
        }
    }

    // Architecture-specific overrides
    let env_var_name = format!("QEMU_{}_BIN", qemu_arch_name.to_uppercase());
    if let Ok(bin) = env::var(&env_var_name) {
        if !bin.is_empty() {
            println!("==> Using {}: {}", env_var_name, bin);
            return Ok(PathBuf::from(bin));
        }
    }

    let use_prebuilt =
        env::var("VIRTMCU_USE_PREBUILT_QEMU").unwrap_or_else(|_| "0".to_string()) == "1";

    let possible_paths = if use_prebuilt {
        vec![
            PathBuf::from(format!(
                "/build/qemu/{}/install/bin/qemu-system-{}",
                build_dir, qemu_arch_name
            )),
            // Fallback non-sanitized
            PathBuf::from(format!(
                "/build/qemu/build-virtmcu/install/bin/qemu-system-{}",
                qemu_arch_name
            )),
        ]
    } else {
        vec![
            workspace_root.join(format!(
                "third_party/qemu/{}/install/bin/qemu-system-{}",
                build_dir, qemu_arch_name
            )),
            workspace_root.join(format!(
                "third_party/qemu/{}/qemu-system-{}",
                build_dir, qemu_arch_name
            )),
            workspace_root.join(format!(
                "third_party/qemu/build-virtmcu/install/bin/qemu-system-{}",
                qemu_arch_name
            )),
            workspace_root.join(format!(
                "third_party/qemu/build-virtmcu/qemu-system-{}",
                qemu_arch_name
            )),
        ]
    };

    for path in possible_paths {
        if path.exists() && path.is_file() {
            return Ok(path);
        }
    }

    Err(anyhow!(
        "QEMU binary for {} not found.\n    virtmcu mandates using the locally built QEMU from third_party/qemu\n    or the prebuilt one in /build/qemu (CI).\n    Please run 'make bootstrap' first.",
        arch
    ))
}

fn has_asan(file: &Path) -> bool {
    if !file.exists() {
        return false;
    }
    if let Ok(out) = Command::new("strings").arg(file).output() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        return stdout.contains("__asan_init") || stdout.contains("__tsan_init");
    }
    false
}

fn check_sanitizer_mismatch(qemu_bin: &Path, module_dir: &Option<PathBuf>) -> Result<()> {
    let mut bin_san = false;
    let mut san_type = "ASan/TSan";

    if has_asan(qemu_bin) {
        bin_san = true;
        if let Ok(out) = Command::new("strings").arg(qemu_bin).output() {
            if String::from_utf8_lossy(&out.stdout).contains("__tsan_init") {
                san_type = "TSan";
            } else {
                san_type = "ASan";
            }
        }
    }

    let mut sample_plugin = None;
    if let Some(dir) = module_dir {
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let fname = path.file_name().unwrap_or_default().to_string_lossy();
                        if fname.starts_with("hw-virtmcu-") && fname.contains(".so") {
                            sample_plugin = Some(path);
                            break;
                        }
                    }
                }
            }
        }
    }

    if let Some(plugin) = sample_plugin {
        let mod_san = has_asan(&plugin);
        if bin_san != mod_san {
            eprintln!(
                "=============================================================================="
            );
            eprintln!("FATAL: {} Instrumentation Mismatch Detected!", san_type);
            eprintln!(
                "------------------------------------------------------------------------------"
            );
            eprintln!(
                "QEMU Binary ({}): Sanitizer={}",
                qemu_bin.display(),
                if bin_san { "YES" } else { "NO" }
            );
            eprintln!(
                "QEMU Modules ({}): Sanitizer={}",
                module_dir.as_ref().unwrap().display(),
                if mod_san { "YES" } else { "NO" }
            );
            eprintln!(
                "------------------------------------------------------------------------------"
            );
            eprintln!("Mixing instrumented and non-instrumented code causes runtime errors");
            eprintln!("or silent corruption.\n");
            if bin_san {
                eprintln!(
                    "Action: Rebuild your plugins with VIRTMCU_USE_ASAN=1 or VIRTMCU_USE_TSAN=1."
                );
            } else {
                eprintln!(
                    "Action: Rebuild your plugins without sanitizers or use an instrumented QEMU."
                );
            }
            eprintln!(
                "=============================================================================="
            );
            return Err(anyhow!("Sanitizer mismatch"));
        }
    }

    Ok(())
}

fn get_qemu_module_dir(workspace_root: &Path, build_dir: &str) -> Option<PathBuf> {
    let use_prebuilt =
        env::var("VIRTMCU_USE_PREBUILT_QEMU").unwrap_or_else(|_| "0".to_string()) == "1";
    if use_prebuilt {
        return Some(PathBuf::from(format!(
            "/build/qemu/{}/install/lib/qemu",
            build_dir
        )));
    }

    let qemu_dir = workspace_root.join("third_party").join("qemu");
    let mut best_path = None;

    let paths = vec![
        qemu_dir
            .join(build_dir)
            .join("install/lib/aarch64-linux-gnu/qemu"),
        qemu_dir
            .join(build_dir)
            .join("install/lib/x86_64-linux-gnu/qemu"),
        qemu_dir.join(build_dir).join("install/lib/qemu"),
    ];

    for path in paths {
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
            best_path = Some(path);
        }
    }

    if best_path.is_none() {
        best_path = Some(qemu_dir.join(build_dir).join("install/lib/qemu"));
    }

    best_path
}

fn main() -> Result<()> {
    let workspace_root = find_workspace_root()?;
    let build_dir = build_dir_name();

    let mut args: Vec<String> = env::args().collect();
    args.remove(0); // skip binary name

    let mut arch = "arm".to_string();
    let mut arch_explicit = false;
    let mut machine = String::new();
    let mut machine_provided = false;
    let mut kernel = None;
    let mut input_file = None;
    let mut input_type = None; // "repl", "yaml", "dts", "dtb"
    let mut extra_args = Vec::new();

    let mut i = 0;
    while i < args.len() {
        if args[i] == "--arch" && i + 1 < args.len() {
            arch = args[i + 1].clone();
            arch_explicit = true;
        }
        i += 1;
    }

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repl" => {
                if i + 1 < args.len() {
                    input_file = Some(args[i + 1].clone());
                    input_type = Some("repl".to_string());
                    i += 2;
                    continue;
                }
            }
            "--yaml" => {
                if i + 1 < args.len() {
                    input_file = Some(args[i + 1].clone());
                    input_type = Some("yaml".to_string());
                    i += 2;
                    continue;
                }
            }
            "--dts" => {
                if i + 1 < args.len() {
                    input_file = Some(args[i + 1].clone());
                    input_type = Some("dts".to_string());
                    i += 2;
                    continue;
                }
            }
            "--dtb" => {
                if i + 1 < args.len() {
                    input_file = Some(args[i + 1].clone());
                    input_type = Some("dtb".to_string());
                    i += 2;
                    continue;
                }
            }
            "--kernel" => {
                if i + 1 < args.len() {
                    kernel = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
            }
            "--machine" => {
                if i + 1 < args.len() {
                    machine = args[i + 1].clone();
                    machine_provided = true;
                    i += 2;
                    continue;
                }
            }
            "--arch" => {
                if i + 1 < args.len() {
                    i += 2;
                    continue;
                }
            }
            _ => {
                extra_args.push(args[i].clone());
                i += 1;
                continue;
            }
        }
        return Err(anyhow!("Missing value for argument {}", args[i]));
    }

    // SOTA: Centralized PYTHONPATH management
    let mut python_path = format!(
        "{}:{}/generated",
        workspace_root.display(),
        workspace_root.display()
    );
    if let Ok(existing) = env::var("PYTHONPATH") {
        if !existing.is_empty() {
            python_path = format!("{}:{}", python_path, existing);
        }
    }
    env::set_var("PYTHONPATH", &python_path);

    if let Some(file) = &input_file {
        if file.ends_with(".yaml") || file.ends_with(".yml") {
            input_type = Some("yaml".to_string());
        } else if file.ends_with(".dts") {
            input_type = Some("dts".to_string());
        } else if file.ends_with(".dtb") {
            input_type = Some("dtb".to_string());
        }
    }

    let mut dtb_path = String::new();
    let mut is_temp_dtb = false;

    let mut _temp_dtb = None;
    let mut _temp_cli = None;
    let mut _temp_arch = None;

    if let Some(ref itype) = input_type {
        let file = input_file.as_ref().unwrap();
        match itype.as_str() {
            "yaml" => {
                println!("Processing virtmcu YAML platform: {}", file);
                let dtb = NamedTempFile::new()?;
                let cli_file = NamedTempFile::new()?;
                let arch_file = NamedTempFile::new()?;

                let mut cmd = if std::process::Command::new("virtmcu-cli")
                    .arg("--version")
                    .output()
                    .is_ok()
                {
                    std::process::Command::new("virtmcu-cli")
                } else {
                    let mut c = std::process::Command::new("cargo");
                    c.arg("run")
                        .arg("-p")
                        .arg("virtmcu-cli")
                        .arg("--release")
                        .arg("--");
                    c
                };

                cmd.arg("platform")
                    .arg("generate")
                    .arg(file)
                    .arg("--out-dtb")
                    .arg(dtb.path())
                    .arg("--out-cli")
                    .arg(cli_file.path())
                    .arg("--out-arch")
                    .arg(arch_file.path());

                if let Ok(router) = env::var("ZENOH_ROUTER") {
                    cmd.arg("--router").arg(router);
                }

                let status = cmd.status()?;
                if !status.success() {
                    return Err(anyhow!("virtmcu-cli platform generate failed"));
                }

                let mut arch_content = String::new();
                std::fs::File::open(arch_file.path())?.read_to_string(&mut arch_content)?;
                let arch_content = arch_content.trim();
                if !arch_content.is_empty() {
                    arch = arch_content.to_string();
                }

                let mut cli_content = String::new();
                std::fs::File::open(cli_file.path())?.read_to_string(&mut cli_content)?;
                for line in cli_content.lines() {
                    if !line.trim().is_empty() {
                        extra_args.push(line.trim().to_string());
                    }
                }

                dtb_path = dtb.path().to_string_lossy().to_string();
                is_temp_dtb = true;
                _temp_dtb = Some(dtb);
                _temp_cli = Some(cli_file);
                _temp_arch = Some(arch_file);
            }
            "dts" => {
                println!("Compiling Device Tree Source: {}", file);
                let dtb = NamedTempFile::new()?;

                let status = Command::new("dtc")
                    .arg("-I")
                    .arg("dts")
                    .arg("-O")
                    .arg("dtb")
                    .arg("-o")
                    .arg(dtb.path())
                    .arg(file)
                    .status()?;
                if !status.success() {
                    return Err(anyhow!("dtc failed"));
                }

                if !arch_explicit {
                    let content = std::fs::read_to_string(file)?;
                    if content.to_lowercase().contains("riscv") {
                        arch = "riscv".to_string();
                    }
                }

                dtb_path = dtb.path().to_string_lossy().to_string();
                is_temp_dtb = true;
                _temp_dtb = Some(dtb);
            }
            "dtb" => {
                dtb_path = file.to_string();
            }
            _ => unreachable!(),
        }
    }

    if !machine_provided {
        if arch == "arm" {
            machine = "arm-generic-fdt".to_string();
        } else if arch.starts_with("riscv") {
            machine = "virt".to_string();
            if !extra_args.contains(&"-bios".to_string()) {
                extra_args.push("-bios".to_string());
                extra_args.push("none".to_string());
            }
        }
    }

    let qemu_bin = resolve_qemu_bin(&arch, &workspace_root, &build_dir)?;

    if !qemu_bin.exists() {
        return Err(anyhow!(
            "❌ ERROR: QEMU binary for {} not found at {}.",
            arch,
            qemu_bin.display()
        ));
    }

    // Check if executable (basic check, could use metadata)
    if let Ok(metadata) = std::fs::metadata(&qemu_bin) {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(anyhow!(
                "❌ ERROR: QEMU binary at {} is not executable.",
                qemu_bin.display()
            ));
        }
    }

    let module_dir = get_qemu_module_dir(&workspace_root, &build_dir);

    check_sanitizer_mismatch(&qemu_bin, &module_dir)?;

    let asan_suffix = if env::var("VIRTMCU_USE_ASAN").unwrap_or_else(|_| "0".to_string()) == "1" {
        "-asan"
    } else {
        ""
    };
    let zenoh_dir = workspace_root.join(format!("third_party/zenoh-c{}", asan_suffix));

    let mut ld_library_paths = vec![];
    if let Ok(val) = env::var("LD_LIBRARY_PATH") {
        if !val.is_empty() {
            ld_library_paths.push(val);
        }
    }

    if zenoh_dir.join("lib").exists() {
        ld_library_paths.insert(0, zenoh_dir.join("lib").to_string_lossy().to_string());
    } else if zenoh_dir.exists() {
        ld_library_paths.insert(0, zenoh_dir.to_string_lossy().to_string());
    }

    if Path::new("/build/zenoh-c/lib").exists() {
        ld_library_paths.insert(0, "/build/zenoh-c/lib".to_string());
    }

    let new_ld_library_path = ld_library_paths.join(":");

    let mut qemu_cmd = Command::new(&qemu_bin);

    if !new_ld_library_path.is_empty() {
        qemu_cmd.env("LD_LIBRARY_PATH", new_ld_library_path);
    }

    if let Some(m_dir) = &module_dir {
        qemu_cmd.env("QEMU_MODULE_DIR", m_dir);
    }

    if let Ok(gcov) = env::var("GCOV_PREFIX") {
        qemu_cmd.env("GCOV_PREFIX", format!("{}/{}", gcov, std::process::id()));
    } else {
        let w_dir = env::var("WORKSPACE_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let qemu_arch_name = match arch.as_str() {
            "riscv" | "riscv64" => "riscv64",
            "riscv32" => "riscv32",
            _ => "arm",
        };
        qemu_cmd.env(
            "GCOV_PREFIX",
            format!(
                "{}/target/coverage/{}_{}_{}",
                w_dir,
                qemu_arch_name,
                build_dir,
                std::process::id()
            ),
        );
    }

    if let Ok(out) = Command::new("ldd").arg(&qemu_bin).output() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = stdout
            .lines()
            .find(|l| l.contains("libasan") || l.contains("libtsan"))
        {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && Path::new(parts[2]).exists() {
                let libsan = parts[2].to_string();
                let mut preload = vec![libsan];
                if let Ok(val) = env::var("LD_PRELOAD") {
                    if !val.is_empty() {
                        preload.push(val);
                    }
                }
                qemu_cmd.env("LD_PRELOAD", preload.join(":"));
            }
        }
    }

    if !machine.is_empty() {
        let mut m_arg = machine.clone();
        if !dtb_path.is_empty() && machine == "arm-generic-fdt" {
            m_arg = format!("{},hw-dtb={}", machine, dtb_path);
            qemu_cmd.arg("-M").arg(m_arg);
        } else {
            qemu_cmd.arg("-M").arg(m_arg);
            if !dtb_path.is_empty() {
                qemu_cmd.arg("-dtb").arg(dtb_path);
            }
        }
    }

    if let Some(k) = kernel {
        qemu_cmd.arg("-kernel").arg(k);
    }

    for arg in extra_args {
        qemu_cmd.arg(arg);
    }

    println!("Running: {:?}", qemu_cmd);

    if is_temp_dtb {
        let mut child = qemu_cmd.spawn()?;

        let mut sigs = nix::sys::signal::SigSet::empty();
        sigs.add(nix::sys::signal::Signal::SIGINT);
        sigs.add(nix::sys::signal::Signal::SIGTERM);

        // Setup simple signal handling to kill QEMU if we are interrupted
        // We can just wait, if parent is killed, QEMU will hopefully die or we just forward signals.
        // Actually, ctrl-c sends SIGINT to the process group, so QEMU receives it automatically.
        let status = child.wait()?;
        std::process::exit(status.code().unwrap_or(1));
    } else {
        let err = qemu_cmd.exec();
        Err(anyhow!("Failed to exec QEMU: {}", err))
    }
}

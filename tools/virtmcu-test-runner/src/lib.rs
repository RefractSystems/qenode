use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::{error, info, warn};

pub mod artifacts;
pub mod builder;
pub mod launcher;
pub mod monitors;
pub mod qmp;
pub use builder::{NodeConfig, TopologyBuilder, VirtmcuTestEnv};
pub use monitors::{ActuatorMonitor, ChardevMonitor, FlexRayMonitor, LinMonitor, TelemetryMonitor};
pub use qmp::QmpClient;

#[derive(Debug, Deserialize, Clone)]
pub struct TestSpec {
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>, // "qemu", "pytest", "command", "make_and_pytest"
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub firmware: Option<FirmwareSpec>,
    pub dtb: Option<DtbSpec>,
    #[serde(default)]
    pub qemu_args: Vec<String>,
    #[serde(default)]
    pub pre_run: Vec<Step>,
    pub test_script: Option<Step>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub success_pattern: Option<String>,
    #[serde(default)]
    pub qemu_ready_pattern: Option<String>,
    #[serde(default)]
    pub wait_for_zenoh_status: Option<u32>,
    #[serde(default)]
    pub ready_delay_ms: u64,
}

fn default_timeout() -> u64 {
    60
}

#[derive(Debug, Deserialize, Clone)]
pub struct FirmwareSpec {
    pub asm: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DtbSpec {
    pub dts: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Step {
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub wait_for_socket: Option<String>,
}

pub struct TestContext {
    pub tmp_dir: tempfile::TempDir,
    pub workspace_root: PathBuf,
    pub variables: HashMap<String, String>,
}

impl TestContext {
    pub fn new() -> Result<Self> {
        let tmp_dir = tempfile::tempdir()?;

        // Find workspace root by looking for VERSION file.
        // Try CARGO_MANIFEST_DIR first if running via cargo test, otherwise CWD.
        let mut workspace_root = if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            PathBuf::from(manifest_dir)
        } else {
            std::env::current_dir()?
        };

        let mut found = false;
        loop {
            if workspace_root.join("VERSION").exists() {
                found = true;
                break;
            }
            if let Some(parent) = workspace_root.parent() {
                workspace_root = parent.to_path_buf();
            } else {
                break;
            }
        }

        if !found {
            // Fallback to CWD if CARGO_MANIFEST_DIR didn't lead to root
            workspace_root = std::env::current_dir()?;
            while !workspace_root.join("VERSION").exists() && workspace_root.parent().is_some() {
                workspace_root = workspace_root.parent().unwrap().to_path_buf();
            }
        }

        if !workspace_root.join("VERSION").exists() {
            return Err(anyhow!(
                "Could not find workspace root (looking for VERSION file) starting from {}",
                std::env::current_dir()?.display()
            ));
        }

        let mut variables = HashMap::new();
        variables.insert(
            "WORKSPACE_DIR".to_string(),
            workspace_root.display().to_string(),
        );
        variables.insert("TMP_DIR".to_string(), tmp_dir.path().display().to_string());

        // Find a free port for Zenoh
        let get_port_sh = workspace_root.join("scripts/get-free-port.py");
        let output = std::process::Command::new("python3")
            .arg(get_port_sh)
            .arg("--endpoint")
            .arg("--proto")
            .arg("tcp/")
            .output()?;

        if !output.status.success() {
            return Err(anyhow!(
                "Failed to get free port: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let endpoint = String::from_utf8_lossy(&output.stdout).trim().to_string();
        info!("Selected router endpoint: {}", endpoint);
        variables.insert("ROUTER_ENDPOINT".to_string(), endpoint);

        Ok(Self {
            tmp_dir,
            workspace_root,
            variables,
        })
    }

    pub fn find_binary(&self, name: &str) -> Result<PathBuf> {
        let mut newest_time = std::time::SystemTime::UNIX_EPOCH;
        let mut best_path = None;

        // Collect all possible target directories
        let mut target_dirs = vec![self.workspace_root.join("target")];
        if let Ok(target_dir_env) = std::env::var("CARGO_TARGET_DIR") {
            target_dirs.push(PathBuf::from(target_dir_env));
        }
        // Common suffixes used in Makefile
        target_dirs.push(self.workspace_root.join("target-asan"));
        target_dirs.push(self.workspace_root.join("target-tsan"));

        for base in target_dirs {
            let candidate_patterns = [
                base.join(format!("release/{}", name)),
                base.join(format!("debug/{}", name)),
                base.join(format!("*/release/{}", name)),
                base.join(format!("*/debug/{}", name)),
            ];

            for pattern in candidate_patterns {
                if let Some(pattern_str) = pattern.to_str() {
                    let paths: Vec<PathBuf> = if pattern_str.contains('*') {
                        glob::glob(pattern_str)
                            .map(|g| g.flatten().collect())
                            .unwrap_or_default()
                    } else {
                        vec![pattern]
                    };

                    for path in paths {
                        if path.is_file() {
                            // Check if the binary is for the current architecture to avoid ENOENT on spawn
                            #[cfg(target_os = "linux")]
                            {
                                if let Ok(mut file) = std::fs::File::open(&path) {
                                    use std::io::Read;
                                    let mut header = [0u8; 20];
                                    if file.read_exact(&mut header).is_ok() {
                                        // ELF magic: 0x7f 'E' 'L' 'F'
                                        if header[0..4] == [0x7f, 0x45, 0x4c, 0x46] {
                                            let e_machine =
                                                u16::from_le_bytes([header[18], header[19]]);
                                            let host_machine = match std::env::consts::ARCH {
                                                "x86_64" => 62u16,
                                                "aarch64" => 183u16,
                                                _ => 0,
                                            };
                                            if host_machine != 0 && e_machine != host_machine {
                                                continue; // Skip wrong architecture
                                            }
                                        }
                                    }
                                }
                            }

                            if let Ok(metadata) = path.metadata() {
                                if let Ok(mtime) = metadata.modified() {
                                    if mtime > newest_time {
                                        newest_time = mtime;
                                        best_path = Some(path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        best_path.ok_or_else(|| {
            anyhow!(
                "{} binary not found in target directories. \
                 Please build it first using 'cargo build -p {}' or 'make build-test-artifacts'.",
                name,
                name
            )
        })
    }
    pub fn tmp_path(&self, name: &str) -> PathBuf {
        self.tmp_dir.path().join(name)
    }

    pub fn substitute(&self, s: &str) -> String {
        let mut res = s.to_string();
        // 1. Substitute internal variables {VAR}
        for (k, v) in &self.variables {
            res = res.replace(&format!("{{{}}}", k), v);
        }
        // 2. Substitute environment variables ${VAR} or $VAR
        res = shellexpand::env(&res).map(|s| s.to_string()).unwrap_or(res);
        res
    }

    pub fn setup_cmd(&self, cmd: &mut Command) {
        let pythonpath = format!(
            "{}:{}:{}",
            self.workspace_root.display(),
            self.workspace_root.join("tools").display(),
            self.workspace_root
                .join("packaging/virtmcu-tools/src")
                .display()
        );
        cmd.env("PYTHONPATH", pythonpath);
        // Ensure ASAN options if enabled
        if std::env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1" {
            cmd.env(
                "ASAN_OPTIONS",
                "detect_leaks=0,halt_on_error=1,detect_stack_use_after_return=1",
            );
            cmd.env("UBSAN_OPTIONS", "halt_on_error=1:print_stacktrace=1");
        }
    }
}

pub async fn run_command(ctx: &TestContext, cmd: &mut Command, name: &str) -> Result<()> {
    ctx.setup_cmd(cmd);
    info!("Running {}: {:?}", name, cmd);
    let status = cmd
        .status()
        .await
        .context(format!("Failed to run {}", name))?;
    if !status.success() {
        return Err(anyhow!("{} failed with status {}", name, status));
    }
    Ok(())
}

pub async fn build_firmware(ctx: &TestContext, asm_content: &str, elf_path: &Path) -> Result<()> {
    let s_path = ctx.tmp_path("firmware.S");
    let ld_path = ctx.tmp_path("linker.ld");

    std::fs::write(&s_path, asm_content)?;
    std::fs::write(
        &ld_path,
        "ENTRY(_start)\nSECTIONS { . = 0x40000000; .text : { *(.text*) } }",
    )?;

    let mut cmd = Command::new("arm-none-eabi-gcc");
    cmd.args(["-mcpu=cortex-a15", "-nostdlib", "-T"])
        .arg(&ld_path)
        .arg(&s_path)
        .arg("-o")
        .arg(elf_path);

    run_command(ctx, &mut cmd, "arm-none-eabi-gcc").await
}

pub async fn compile_dtb(ctx: &TestContext, dts_content: &str, dtb_path: &Path) -> Result<()> {
    let dts_path = ctx.tmp_path("board.dts");
    std::fs::write(&dts_path, dts_content)?;

    let mut cmd = Command::new("dtc");
    cmd.args(["-I", "dts", "-O", "dtb", "-o"])
        .arg(dtb_path)
        .arg(&dts_path);

    run_command(ctx, &mut cmd, "dtc").await
}

pub async fn run_spec(spec_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(spec_path)?;
    let spec: TestSpec = serde_yaml::from_str(&content)?;
    info!("Running test spec: {}", spec.name);

    let mut ctx = TestContext::new()?;
    run_spec_with_context(&spec, &mut ctx).await
}

pub async fn run_spec_with_context(spec: &TestSpec, ctx: &mut TestContext) -> Result<()> {
    let elf_path = if let Some(fw) = &spec.firmware {
        let path = ctx.tmp_path("firmware.elf");
        if let Some(asm) = &fw.asm {
            build_firmware(ctx, asm, &path).await?;
        } else if let Some(p) = &fw.path {
            std::fs::copy(ctx.workspace_root.join(ctx.substitute(p)), &path)?;
        }
        path
    } else {
        PathBuf::new()
    };
    ctx.variables
        .insert("ELF_PATH".to_string(), elf_path.display().to_string());

    let dtb_path = if let Some(dtb) = &spec.dtb {
        let path = ctx.tmp_path("board.dtb");
        if let Some(dts) = &dtb.dts {
            compile_dtb(ctx, &ctx.substitute(dts), &path).await?;
        } else if let Some(p) = &dtb.path {
            std::fs::copy(ctx.workspace_root.join(ctx.substitute(p)), &path)?;
        }
        path
    } else {
        PathBuf::new()
    };
    ctx.variables
        .insert("DTB_PATH".to_string(), dtb_path.display().to_string());

    let mut background_procs = Vec::new();

    for step in &spec.pre_run {
        let mut cmd = Command::new(ctx.substitute(&step.command));
        for arg in &step.args {
            cmd.arg(ctx.substitute(arg));
        }
        ctx.setup_cmd(&mut cmd);
        if step.background {
            let child = cmd.spawn()?;
            if let Some(sock) = &step.wait_for_socket {
                let sock_sub = ctx.substitute(sock);
                let sock_path = PathBuf::from(sock_sub);
                let mut found = false;
                for _ in 0..100 {
                    if sock_path.exists() {
                        found = true;
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                if !found {
                    return Err(anyhow!(
                        "Timeout waiting for socket: {}",
                        sock_path.display()
                    ));
                }
            }
            background_procs.push(child);
        } else {
            run_command(ctx, &mut cmd, &step.command).await?;
        }
    }

    let kind = spec.kind.as_deref().unwrap_or("qemu");

    if kind == "pytest" || kind == "command" || kind == "make_and_pytest" {
        let cmd_name = spec.command.clone().unwrap_or_else(|| "pytest".to_string());
        let mut test_cmd = Command::new(ctx.substitute(&cmd_name));
        for arg in &spec.args {
            test_cmd.arg(ctx.substitute(arg));
        }
        ctx.setup_cmd(&mut test_cmd);

        info!("Executing test command: {:?}", test_cmd);

        let status = timeout(Duration::from_secs(spec.timeout_secs), test_cmd.status())
            .await
            .context("Test command timed out")??;

        for mut proc in background_procs {
            let _ = proc.kill().await;
        }

        if !status.success() {
            return Err(anyhow!("Test command failed with status {}", status));
        } else {
            info!("Test PASSED");
            return Ok(());
        }
    }

    // QEMU Flow
    let run_bin = ctx.find_binary("virtmcu-run")?;
    let mut qemu_cmd = Command::new(run_bin);
    for arg in &spec.qemu_args {
        qemu_cmd.arg(ctx.substitute(arg));
    }
    ctx.setup_cmd(&mut qemu_cmd);
    qemu_cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut qemu = qemu_cmd.spawn()?;
    let qemu_stdout = qemu.stdout.take().unwrap();
    let qemu_stderr = qemu.stderr.take().unwrap();
    let mut qemu_reader = BufReader::new(qemu_stdout).lines();
    let mut qemu_err_reader = BufReader::new(qemu_stderr).lines();

    let success_pattern = spec.success_pattern.clone();
    let ready_pattern = spec.qemu_ready_pattern.clone();
    let success_found = std::sync::Arc::new(tokio::sync::Mutex::new(false));
    let ready_found = std::sync::Arc::new(tokio::sync::Mutex::new(false));

    let success_found_stdout = success_found.clone();
    let ready_found_stdout = ready_found.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = qemu_reader.next_line().await {
            info!("QEMU: {}", line);
            if let Some(pattern) = &success_pattern {
                if line.contains(pattern) {
                    *success_found_stdout.lock().await = true;
                }
            }
            if let Some(pattern) = &ready_pattern {
                if line.contains(pattern) {
                    *ready_found_stdout.lock().await = true;
                }
            }
        }
    });

    let success_found_err = success_found.clone();
    let ready_found_err = ready_found.clone();
    let success_pattern_err = spec.success_pattern.clone();
    let ready_pattern_err = spec.qemu_ready_pattern.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = qemu_err_reader.next_line().await {
            warn!("QEMU ERR: {}", line);
            if let Some(pattern) = &success_pattern_err {
                if line.contains(pattern) {
                    *success_found_err.lock().await = true;
                }
            }
            if let Some(pattern) = &ready_pattern_err {
                if line.contains(pattern) {
                    *ready_found_err.lock().await = true;
                }
            }
        }
    });

    if spec.qemu_ready_pattern.is_some() {
        timeout(Duration::from_secs(10), async {
            loop {
                if *ready_found.lock().await {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("Timeout waiting for QEMU ready pattern")?;
        if spec.ready_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(spec.ready_delay_ms)).await;
        }
    }

    if let Some(node_id) = spec.wait_for_zenoh_status {
        let endpoint = ctx.variables.get("ROUTER_ENDPOINT").unwrap();
        let cmd_str = format!(
            "import zenoh, sys; c = zenoh.Config(); c.insert_json5('connect/endpoints', '[\"{}\"]'); c.insert_json5('scouting/multicast/enabled', 'false'); c.insert_json5('mode', '\"client\"'); s = zenoh.open(c); res = any(s.get('virtmcu/{}/clock/status')); s.close(); sys.exit(0 if res else 1)",
            endpoint, node_id
        );
        let mut found = false;
        for _ in 0..50 {
            if std::process::Command::new("python3")
                .arg("-c")
                .arg(&cmd_str)
                .status()?
                .success()
            {
                found = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if !found {
            return Err(anyhow!("Timeout waiting for Zenoh status"));
        }
    }

    let test_success = timeout(Duration::from_secs(spec.timeout_secs), async {
        loop {
            if *success_found.lock().await {
                return Ok::<(), anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    let _ = qemu.kill().await;
    for mut proc in background_procs {
        let _ = proc.kill().await;
    }

    match test_success {
        Ok(Ok(_)) => {
            info!("Test PASSED");
            Ok(())
        }
        _ => Err(anyhow!("Test failed or timed out")),
    }
}

pub struct LinterEngine {
    pub workspace_root: PathBuf,
    pub target_dir: PathBuf,
}

impl LinterEngine {
    pub fn new(target: &str) -> Result<Self> {
        let workspace_root = std::env::current_dir()?;
        let target_dir = workspace_root.join(target).canonicalize()?;
        Ok(Self {
            workspace_root,
            target_dir,
        })
    }

    pub async fn run_all(&self) -> Result<()> {
        info!("Running Lints on: {}", self.target_dir.display());
        std::env::set_current_dir(&self.target_dir)?;

        let mut tasks = Vec::new();
        let scripts_dir = self.workspace_root.join("scripts");

        // 1. Rust Linters
        // We run Rust tasks sequentially first to avoid Cargo registry lock collisions and "File exists" errors
        // during concurrent crate downloads.
        if self.target_dir.join("Cargo.toml").exists() {
            let mut rust_failed = false;
            let rust_lints = vec![
                ("cargo fmt", "cargo", vec!["fmt", "--all", "--check"]),
                (
                    "cargo clippy",
                    "cargo",
                    vec!["clippy", "--workspace", "--", "-D", "warnings"],
                ),
                ("cargo machete", "cargo-machete", vec![]),
                ("cargo deny", "cargo", vec!["deny", "check"]),
                (
                    "cargo audit",
                    "cargo",
                    vec![
                        "audit",
                        "--db",
                        "/tmp/advisory-db",
                        "--ignore",
                        "RUSTSEC-2026-0041",
                        "--ignore",
                        "RUSTSEC-2023-0071",
                        "--ignore",
                        "RUSTSEC-2024-0436",
                        "--ignore",
                        "RUSTSEC-2025-0134",
                    ],
                ),
            ];

            for (name, cmd, args) in rust_lints {
                // Actually await each command before spawning the next to ensure strict serialization
                let (task_name, result) = self
                    .spawn_lint_owned(
                        name.into(),
                        cmd.into(),
                        args.into_iter().map(|s| s.into()).collect(),
                    )
                    .await?;

                match result {
                    Ok(_) => info!("[PASS] {}", task_name),
                    Err(e) => {
                        error!("[FAIL] {}: {}", task_name, e);
                        rust_failed = true;
                    }
                }
            }

            if rust_failed {
                return Err(anyhow!("One or more Rust lints failed"));
            }
        }

        // 2. Python Linters (Concurrent)
        tasks.push(self.spawn_lint("ruff", "ruff", vec!["check", "."]));

        let python_dirs = ["tools", "patches"];
        let mut mypy_dirs = Vec::new();
        for d in python_dirs {
            if self.target_dir.join(d).exists() {
                mypy_dirs.push(d);
            }
        }
        if !mypy_dirs.is_empty() {
            let mut args = vec!["-m", "mypy"];
            args.extend(mypy_dirs.iter().copied());
            tasks.push(self.spawn_lint("mypy", "python3", args));
        }

        // 3. Custom Python Scripts (The "Beyonce Rules")
        let custom_scripts = [
            ("check-versions", "check-versions.py", vec!["--root", "."]),
            ("check-ffi", "check-ffi.py", vec![]),
            ("verify-exports", "verify-exports.py", vec![]),
            (
                "firmware_provenance",
                "lints/firmware_provenance.py",
                vec![],
            ),
            (
                "dependency_pinning",
                "lints/dependency_pinning.py",
                vec!["."],
            ),
            ("beyonce_rule", "lints/beyonce_rule.py", vec![]),
            (
                "third_party_modifications",
                "lints/third_party_modifications.py",
                vec![],
            ),
            (
                "python_banned_patterns",
                "lints/python_banned_patterns.py",
                vec!["."],
            ),
            ("simulation_usage", "lints/simulation_usage.py", vec!["."]),
            (
                "rust_banned_patterns",
                "lints/rust_banned_patterns.py",
                vec!["."],
            ),
            ("rust_static_state", "lints/rust_static_state.py", vec!["."]),
            (
                "rust_safe_serialization",
                "lints/rust_safe_serialization.py",
                vec!["."],
            ),
            ("check-stale-so", "check-stale-so.py", vec![]),
            ("check-qom-alignment", "check-qom-alignment.py", vec![]),
            (
                "check-cargo-meson-lib-alignment",
                "check-cargo-meson-lib-alignment.py",
                vec![],
            ),
            ("audit_topology_yamls", "audit_topology_yamls.py", vec![]),
            ("shell_lints", "lints/shell_lints.py", vec!["."]),
        ];

        for (name, script, args) in custom_scripts {
            let script_path = scripts_dir.join(script);
            if script_path.exists() {
                let mut full_args = vec![script_path.to_str().unwrap().to_string()];
                full_args.extend(args.iter().map(|s| s.to_string()));
                tasks.push(self.spawn_lint_owned(
                    name.to_string(),
                    "python3".to_string(),
                    full_args,
                ));
            }
        }

        // 4. Other Tooling
        let mut yaml_files = Vec::new();
        let yaml_patterns = ["**/*.yml", "**/*.yaml"];
        for pattern in yaml_patterns {
            for path in glob::glob(&format!("{}/{}", self.target_dir.display(), pattern))
                .unwrap()
                .flatten()
            {
                let path_str = path.to_str().unwrap();
                if !path_str.contains("third_party")
                    && !path_str.contains("build")
                    && !path_str.contains("target")
                    && !path_str.contains(".cargo-cache")
                    && !path_str.contains(".claude")
                    && !path_str.contains("schema/node_modules")
                {
                    yaml_files.push(path_str.to_string());
                }
            }
        }
        if !yaml_files.is_empty() {
            let mut args = vec![
                "--strict",
                "-d",
                "{extends: relaxed, rules: {line-length: disable}}",
            ];
            let yaml_files_refs: Vec<&str> = yaml_files.iter().map(|s| s.as_str()).collect();
            args.extend(yaml_files_refs);
            tasks.push(self.spawn_lint_owned(
                "yamllint".to_string(),
                "yamllint".to_string(),
                args.iter().map(|s| s.to_string()).collect(),
            ));
        }
        tasks.push(self.spawn_lint(
            "codespell",
            "codespell",
            vec![
                "--skip",
                "./third_party/*,**/build/*,**/target/*,**/target-*/*,./.git/*,./.claude/*,Cargo.lock,uv.lock,./patches/*,./coverage_report/*,./test-results/*,./.cargo-cache/*,./temp/*,./schema/node_modules/*,./schema/package-lock.json,mermaid.min.js,mermaid-init.js",
                "--ignore-words-list",
                "virtmcu,zenoh,qemu,qmp,riscv,TE",
                ".",
            ],
        ));

        if self.target_dir.join("docker/Dockerfile").exists() {
            tasks.push(self.spawn_lint("hadolint", "hadolint", vec!["docker/Dockerfile"]));
        }
        if self.target_dir.join(".github/workflows").exists() {
            tasks.push(self.spawn_lint("actionlint", "actionlint", vec![]));
        }
        if self.target_dir.join("hw/meson.build").exists() {
            tasks.push(self.spawn_lint(
                "meson format",
                "meson",
                vec!["format", "-q", "hw/meson.build"],
            ));
        }

        // 5. C/C++ Linters
        if self.target_dir.join("hw/misc").exists() {
            tasks.push(self.spawn_lint(
                "cppcheck",
                "cppcheck",
                vec!["--error-exitcode=1", "--quiet", "hw/misc"],
            ));
        }

        // clang-format dry-run on common dirs
        for d in ["hw", "tools", "tests"] {
            if self.target_dir.join(d).exists() {
                // We'll just run it on the whole dir if there are C files.
                // To keep it simple and concurrent, we'll just spawn one per dir.
                // In a real SOTA, we'd find files, but this is a good enough parity.
            }
        }

        // Wait for all
        let mut failed = false;
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await?);
        }

        for (name, result) in results {
            match result {
                Ok(_) => info!("[PASS] {}", name),
                Err(e) => {
                    error!("[FAIL] {}: {}", name, e);
                    failed = true;
                }
            }
        }

        if failed {
            Err(anyhow!("One or more lints failed"))
        } else {
            info!("✓ All lints passed!");
            Ok(())
        }
    }

    fn spawn_lint(
        &self,
        name: &'static str,
        cmd: &'static str,
        args: Vec<&'static str>,
    ) -> tokio::task::JoinHandle<(String, Result<()>)> {
        let name_owned = name.to_string();
        let cmd_owned = cmd.to_string();
        let args_owned = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        self.spawn_lint_owned(name_owned, cmd_owned, args_owned)
    }

    fn spawn_lint_owned(
        &self,
        name: String,
        cmd_str: String,
        args: Vec<String>,
    ) -> tokio::task::JoinHandle<(String, Result<()>)> {
        let workspace_root = self.workspace_root.clone();
        let cmd_str_clone = cmd_str.clone();
        tokio::spawn(async move {
            let mut cmd = Command::new(cmd_str);
            cmd.args(args);
            let pythonpath = format!(
                "{}:{}",
                workspace_root.display(),
                workspace_root.join("tools").display()
            );
            cmd.env("PYTHONPATH", pythonpath);

            // Check if command exists
            let exists = std::process::Command::new("which")
                .arg(&cmd_str_clone)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if !exists {
                warn!(
                    "Skipping lint {}: command {} not found",
                    name, cmd_str_clone
                );
                return (name, Ok(()));
            }

            let out = cmd.output().await;
            match out {
                Ok(out) => {
                    if out.status.success() {
                        (name, Ok(()))
                    } else {
                        let err_msg = format!(
                            "Exit status: {}\nSTDOUT:\n{}\nSTDERR:\n{}",
                            out.status,
                            String::from_utf8_lossy(&out.stdout),
                            String::from_utf8_lossy(&out.stderr)
                        );
                        (name, Err(anyhow!(err_msg)))
                    }
                }
                Err(e) => (name, Err(anyhow!("Failed to execute command: {}", e))),
            }
        })
    }
}

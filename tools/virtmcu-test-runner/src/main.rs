use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};
use virtmcu_test_runner::{run_spec, LinterEngine};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    cmd: SubCommand,
}

#[derive(Subcommand, Debug)]
enum SubCommand {
    /// Run tests (reads yaml specs)
    Run {
        /// Test tier to run (e.g., integration, unit). If specified, runs all specs in tests/specs/<tier>/
        #[arg(short, long)]
        tier: Option<String>,

        /// Path to a specific test specification YAML or directory
        #[arg(short, long)]
        spec: Option<PathBuf>,

        /// Run a specific built-in test (for migration phase)
        #[arg(long)]
        test: Option<String>,

        /// Enable AddressSanitizer (ASan) for integration tests
        #[arg(long)]
        asan: bool,

        /// Domain filter for integration tests (runs specific test file)
        #[arg(short, long)]
        domain: Option<String>,
    },
    /// Run lints
    Lint {
        /// Target directory to lint
        #[arg(short, long, default_value = ".")]
        target: String,
    },
    /// Run coverage (Python + Rust)
    Coverage {
        /// Run integration coverage (guest-side)
        #[arg(long)]
        integration: bool,
        /// Run peripheral coverage (host-side C)
        #[arg(long)]
        peripheral: bool,
        /// Optional data directory for peripheral coverage
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Run Miri (undefined behavior detection)
    Miri,
}

#[tokio::main]
async fn main() -> Result<()> {
    #[derive(Debug)]
    struct DummyVTimeProvider;
    impl virtmcu_observability::processors::VTimeProvider for DummyVTimeProvider {
        fn current_vtime_ns(&self) -> u64 {
            0
        }
    }
    let _telemetry = virtmcu_observability::init_telemetry(
        "virtmcu-test-runner",
        std::sync::Arc::new(DummyVTimeProvider),
    );
    let args = Args::parse();

    match args.cmd {
        SubCommand::Run {
            tier,
            spec,
            asan,
            domain,
            ..
        } => {
            if let Some(tier_name) = tier {
                match tier_name.as_str() {
                    "unit" => {
                        let mut cmd = std::process::Command::new("cargo");
                        cmd.arg("test")
                            .arg("--workspace")
                            .arg("--exclude")
                            .arg("native-integration");

                        // Inject VIRTMCU_UNIT_TEST to enable stubs in virtmcu-qom
                        cmd.env("VIRTMCU_UNIT_TEST", "1");

                        // Inject RUSTFLAGS to allow missing symbols during plugin unit tests
                        if let Ok(flags) = std::env::var("RUSTFLAGS") {
                            cmd.env(
                                "RUSTFLAGS",
                                format!(
                                    "{} -C link-arg=-Wl,--unresolved-symbols=ignore-all",
                                    flags
                                ),
                            );
                        } else {
                            cmd.env(
                                "RUSTFLAGS",
                                "-C link-arg=-Wl,--unresolved-symbols=ignore-all",
                            );
                        }

                        let status = cmd.status()?;
                        if !status.success() {
                            return Err(anyhow!("Unit tests failed"));
                        }
                    }
                    "integration" => {
                        let mut extra_env = Vec::new();
                        if asan {
                            info!("==> Enabling ASan for integration tests...");
                            extra_env.push(("VIRTMCU_USE_ASAN", "1"));
                            extra_env.push(("VIRTMCU_STALL_TIMEOUT_MS", "300000"));
                            extra_env.push((
                                "ASAN_OPTIONS",
                                "detect_leaks=0,halt_on_error=1,detect_stack_use_after_return=1",
                            ));
                            extra_env.push(("UBSAN_OPTIONS", "halt_on_error=1:print_stacktrace=1"));
                        }

                        let mut cmd = std::process::Command::new("cargo");
                        cmd.arg("+nightly")
                            .arg("test")
                            .arg("-Z")
                            .arg("bindeps")
                            .arg("-p")
                            .arg("native-integration");
                        if let Some(domain_name) = domain {
                            if domain_name != "all" {
                                cmd.arg("--test").arg(domain_name);
                            }
                        }

                        let status = cmd.status()?;
                        if !status.success() {
                            return Err(anyhow!("Integration tests failed"));
                        }
                    }
                    _ => {
                        let spec_dir = std::env::current_dir()?.join("tests/specs").join(tier_name);
                        if !spec_dir.exists() {
                            return Err(anyhow!(
                                "Tier directory not found: {}",
                                spec_dir.display()
                            ));
                        }
                        run_specs_in_dir(&spec_dir, asan).await?;
                    }
                }
            } else if let Some(spec_path) = spec {
                if spec_path.is_dir() {
                    run_specs_in_dir(&spec_path, asan).await?;
                } else {
                    run_spec(&spec_path, asan).await?;
                }
            } else {
                info!("No test specified. Use --tier <tier> or --spec <path>");
            }
        }
        SubCommand::Lint { target } => {
            let engine = LinterEngine::new(&target)?;
            engine.run_all().await?;
        }
        SubCommand::Coverage {
            integration,
            peripheral,
            data_dir,
        } => {
            if integration {
                run_integration_coverage()?;
            } else if peripheral {
                run_peripheral_coverage(data_dir)?;
            } else {
                run_unit_coverage()?;
            }
        }
        SubCommand::Miri => {
            info!("==> Running Miri (Undefinded Behavior Detection)...");
            let mut cmd = std::process::Command::new("cargo");
            cmd.arg("+nightly")
                .arg("miri")
                .arg("test")
                .arg("-p")
                .arg("virtmcu-api")
                .arg("-p")
                .arg("virtmcu-test-runner")
                .arg("-p")
                .arg("zenoh_coordinator")
                .arg("-p")
                .arg("deterministic_coordinator")
                .arg("-p")
                .arg("virtmcu-zenoh-config");

            cmd.env("MIRIFLAGS", "-Zmiri-disable-isolation");

            let status = cmd.status()?;
            if !status.success() {
                return Err(anyhow!("Miri tests failed"));
            }
            info!("✓ Miri tests passed.");
        }
    }

    Ok(())
}

fn run_unit_coverage() -> Result<()> {
    info!("==> Running Unit Coverage (Rust)...");

    // 2. Rust Coverage (tarpaulin)
    let cargo_version = std::process::Command::new("cargo")
        .arg("tarpaulin")
        .arg("--version")
        .output();

    if cargo_version.is_ok() && cargo_version.unwrap().status.success() {
        let mut rust_cmd = std::process::Command::new("cargo");
        rust_cmd.args([
            "tarpaulin",
            "--workspace",
            "--exclude",
            "native-integration",
            "--out",
            "Xml",
            "--output-dir",
            "test-results/",
            "--engine",
            "llvm",
        ]);
        rust_cmd.env("VIRTMCU_UNIT_TEST", "1");
        rust_cmd.env(
            "RUSTFLAGS",
            "-C link-arg=-Wl,--unresolved-symbols=ignore-all",
        );

        let rust_status = rust_cmd.status()?;
        if !rust_status.success() {
            return Err(anyhow!("Rust coverage failed"));
        }
        info!("✓ Coverage reports generated in test-results/");
    } else {
        warn!("Skipping Rust coverage: cargo-tarpaulin is not installed. (Run `cargo install cargo-tarpaulin`)");
    }
    Ok(())
}

fn run_integration_coverage() -> Result<()> {
    info!("==> Running Integration Coverage (drcov + virtmcu-coverage)...");

    // Parity with run-integration-coverage.sh
    std::process::Command::new("make")
        .args(["-C", "tests/fixtures/guest_apps/boot_arm"])
        .status()?;

    // Find drcov plugin
    let search_paths = [
        "/usr/local/lib/qemu/plugins",
        "/build/qemu",
        "third_party/qemu/build-virtmcu",
    ];
    let mut drcov_so = None;
    for p in search_paths {
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                if entry.path().is_file() && entry.file_name() == "libdrcov.so" {
                    drcov_so = Some(entry.path());
                    break;
                }
            }
        }
        if drcov_so.is_some() {
            break;
        }
    }

    let drcov_so = drcov_so.ok_or_else(|| anyhow!("libdrcov.so not found"))?;
    info!("==> Using drcov plugin: {}", drcov_so.display());

    std::fs::create_dir_all("coverage-data")?;

    std::process::Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "virtmcu-run",
            "-p",
            "virtmcu-coverage",
        ])
        .status()?;

    // Run with drcov plugin
    let mut qemu = std::process::Command::new("./target/release/virtmcu-run")
        .args([
            "--dtb",
            "tests/fixtures/guest_apps/boot_arm/minimal.dtb",
            "--kernel",
            "tests/fixtures/guest_apps/boot_arm/hello.elf",
            "-nographic",
            "-m",
            "128M",
            "-display",
            "none",
            "-plugin",
            &format!("{},filename=coverage-data/hello.drcov", drcov_so.display()),
            "-d",
            "plugin",
        ])
        .spawn()?;

    std::thread::sleep(std::time::Duration::from_secs(2));

    #[cfg(unix)]
    {
        use libc::{kill, SIGINT};
        unsafe {
            kill(qemu.id() as i32, SIGINT);
        }
    }

    let _ = qemu.wait()?;

    if !std::path::Path::new("coverage-data/hello.drcov").exists() {
        return Err(anyhow!("hello.drcov was not created"));
    }

    let analyze_status = std::process::Command::new("./target/release/virtmcu-coverage")
        .args([
            "coverage-data/hello.drcov",
            "tests/fixtures/guest_apps/boot_arm/hello.elf",
            "--fail-under",
            "80",
        ])
        .status()?;

    if !analyze_status.success() {
        return Err(anyhow!("Coverage analysis failed"));
    }

    Ok(())
}

fn run_peripheral_coverage(data_dir: Option<PathBuf>) -> Result<()> {
    info!("==> Running Peripheral Coverage (gcovr)...");

    let cov_dir = data_dir.unwrap_or_else(|| PathBuf::from("/workspace/all-coverage"));
    std::fs::create_dir_all("test-results")?;
    std::fs::create_dir_all(&cov_dir)?;

    let virtmcu_src = if Path::new("/build/qemu/hw/virtmcu").exists() {
        "/build/qemu/hw/virtmcu".to_string()
    } else if Path::new("third_party/qemu/hw/virtmcu").exists() {
        "third_party/qemu/hw/virtmcu".to_string()
    } else {
        return Err(anyhow!("virtmcu source directory not found"));
    };

    let qemu_build = if Path::new("/build/qemu/build-virtmcu").exists() {
        "/build/qemu/build-virtmcu".to_string()
    } else if Path::new("third_party/qemu/build-virtmcu").exists() {
        "third_party/qemu/build-virtmcu".to_string()
    } else {
        return Err(anyhow!("QEMU build directory not found"));
    };

    let status = std::process::Command::new("gcovr")
        .args([
            "-r",
            &virtmcu_src,
            "--gcov-executable",
            "gcov",
            "--gcov-ignore-errors=no_working_dir_found",
            "--object-directory",
            &qemu_build,
            "--xml",
            "test-results/peripheral-coverage.xml",
            "--html-details",
            "test-results/peripheral-coverage.html",
            "--print-summary",
            cov_dir.to_str().unwrap(),
        ])
        .status()?;

    if !status.success() {
        return Err(anyhow!("gcovr failed"));
    }

    Ok(())
}

async fn run_specs_in_dir(dir: &Path, use_asan: bool) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;
    let mut results = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            info!("Found spec: {}", path.display());
            match run_spec(&path, use_asan).await {
                Ok(_) => results.push((path, true, None)),
                Err(e) => results.push((path, false, Some(e))),
            }
        }
    }
    info!("--- Test Results ---");
    let mut failed = 0;
    for (path, success, err) in results {
        if success {
            info!("PASS: {}", path.display());
        } else {
            error!("FAIL: {} - {:?}", path.display(), err);
            failed += 1;
        }
    }
    if failed > 0 {
        return Err(anyhow!("{} tests failed", failed));
    }
    Ok(())
}

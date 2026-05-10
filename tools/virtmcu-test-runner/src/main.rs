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
    },
    /// Run lints
    Lint {
        /// Target directory to lint
        #[arg(short, long, default_value = ".")]
        target: String,
    },
    /// Run coverage (Python + Rust)
    Coverage,
    /// Run Miri (undefined behavior detection)
    Miri,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    match args.cmd {
        SubCommand::Run { tier, spec, .. } => {
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
                        info!("==> Building zenoh_coordinator...");
                        let mut build_cmd = std::process::Command::new("cargo");
                        build_cmd.arg("build").arg("-p").arg("zenoh_coordinator");
                        let build_status = build_cmd.status()?;
                        if !build_status.success() {
                            return Err(anyhow!("Failed to build zenoh_coordinator"));
                        }

                        let mut cmd = std::process::Command::new("cargo");
                        cmd.arg("test").arg("-p").arg("native-integration");
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
                        run_specs_in_dir(&spec_dir).await?;
                    }
                }
            } else if let Some(spec_path) = spec {
                if spec_path.is_dir() {
                    run_specs_in_dir(&spec_path).await?;
                } else {
                    run_spec(&spec_path).await?;
                }
            } else {
                info!("No test specified. Use --tier <tier> or --spec <path>");
            }
        }
        SubCommand::Lint { target } => {
            let engine = LinterEngine::new(&target)?;
            engine.run_all().await?;
        }
        SubCommand::Coverage => {
            info!("==> Running Coverage (Python + Rust)...");

            // 1. Python Coverage
            if std::path::Path::new("tests/unit/").exists() {
                let py_status = std::process::Command::new("pytest")
                    .args([
                        "tests/unit/",
                        "-v",
                        "--cov=packaging/virtmcu-tools",
                        "--cov-report=xml:test-results/python-unit-coverage.xml",
                    ])
                    .status();

                if let Ok(status) = py_status {
                    if !status.success() {
                        warn!("Python coverage failed or some tests failed");
                    }
                } else {
                    warn!("Failed to execute pytest. Is it installed?");
                }
            } else {
                info!("Skipping Python coverage: tests/unit/ directory not found.");
            }

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

async fn run_specs_in_dir(dir: &Path) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;
    let mut results = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("yaml") {
            info!("Found spec: {}", path.display());
            match run_spec(&path).await {
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

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::path::Path;
use xshell::{cmd, Shell};

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "VirtMCU workspace automation tasks", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the workspace
    Bootstrap,
    /// Synchronize dependency versions
    SyncVersions,
    /// Build QEMU and VirtMCU plugins
    BuildQemu,
    /// Build Zenoh-C
    BuildZenohC,
    /// Build FlatCC
    BuildFlatcc,
    /// Build all third-party dependencies
    BuildThirdParty,
    /// Build rust modules
    BuildRustModules {
        rust_dir: String,
        target_dir: String,
        out_dir: String,
        artifacts: Vec<String>,
    },
    /// Gen module trigger
    GenModuleTrigger {
        out: String,
        #[arg(long)]
        obj: Option<String>,
        #[arg(long)]
        extra_inc: Option<String>,
        #[arg(long)]
        extra_c: Option<String>,
        objs: Vec<String>,
    },
    /// Alias for build-qemu
    Build,
    /// Build all test artifacts across all domains
    BuildTestArtifacts,
    /// CI Targets
    CiCheck,
    CiLint,
    CiUnit,
    CiUnitCoverage,
    CiUnitMiri,
    CiIntegration {
        #[arg(long, short)]
        domain: Option<String>,
    },
    CiIntegrationCoverage,
    CiIntegrationAsan {
        #[arg(long, short)]
        domain: Option<String>,
    },
    CiPeripheralCoverage,
    CiBuildThirdParty,
    CiBuildThirdPartyAsan,
    CiFull,
    /// Dev Targets
    SetupDev,
    DevAll,
    DevCheck,
    DevLint,
    DevUnit,
    DevUnitCoverage,
    DevUnitMiri,
    DevIntegration {
        #[arg(long, short)]
        domain: Option<String>,
    },
    DevIntegrationCoverage,
    DevIntegrationAsan {
        #[arg(long, short)]
        domain: Option<String>,
    },
    DevPeripheralCoverage,
    /// Install git hooks
    InstallGitHooks,
    /// Format all codebase
    FmtAll,
    FmtPython,
    FmtRust,
    FmtMeson,
    FmtC,
    FmtYaml,
    /// Docker Targets
    DockerDev,
    DockerAll,
    DockerBase,
    DockerToolchain,
    DockerDevenv,
    ThirdPartyBuilder,
    DockerCi,
    DockerCiAsan,
    DockerRuntime,
    SmokeBase,
    SmokeToolchain,
    SmokeDevenv,
    SmokeCi,
    SmokeCiAsan,
    /// Clean up targets
    CleanSim,
    DeleteProfraw,
    CleanDebug,
    Clean,
    Distclean,
    /// Run emulator
    Run {
        #[arg(last = true)]
        extra_args: Vec<String>,
    },
    /// Build and serve mdBook
    Book,
    BookServe,
    /// Release tagging
    Tag {
        #[arg(long, short)]
        version: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let sh = Shell::new()?;

    // Load BUILD_DEPS
    if let Ok(build_deps) = fs::read_to_string("BUILD_DEPS") {
        for line in build_deps.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                if let Some((k, v)) = line.split_once('=') {
                    sh.set_var(k, v);
                }
            }
        }
    }

    // Determine ARCH
    let arch = if env::consts::ARCH == "x86_64" {
        "amd64"
    } else if env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        env::consts::ARCH
    };
    sh.set_var("ARCH", arch);

    // Calculate PATCHES_HASH
    let hash_output = cmd!(
        sh,
        "bash -c 'cat BUILD_DEPS $(find patches -type f | sort) | sha256sum | head -c 12'"
    )
    .read()
    .unwrap_or_else(|_| "unknown".to_string());
    sh.set_var("PATCHES_HASH", &hash_output);

    let qemu_version = sh.var("QEMU_VERSION").unwrap_or_default();
    sh.set_var(
        "THIRD_PARTY_CACHE_TAG",
        format!("{}-{}", qemu_version, hash_output),
    );

    // Determine IMAGE_TAG
    let image_tag = if let Ok(tag) = env::var("IMAGE_TAG") {
        tag
    } else if let Ok(exact_tag) = cmd!(sh, "git describe --tags --exact-match").ignore_stderr().read() {
        exact_tag.trim_start_matches('v').to_string()
    } else if env::var("CI").unwrap_or_default() == "true" {
        let hash = cmd!(sh, "git rev-parse --short HEAD")
            .read()
            .unwrap_or_else(|_| "unknown".to_string());
        format!("sha-{}", hash)
    } else {
        "latest".to_string()
    };
    sh.set_var("IMAGE_TAG", &image_tag);

    // Setup variables
    let registry = sh.var("VIRTMCU_IMAGE_REGISTRY").unwrap_or_default();
    let devenv_img = sh.var("VIRTMCU_DEVENV_IMAGE").unwrap_or_default();
    let ci_img = sh.var("VIRTMCU_CI_IMAGE").unwrap_or_default();

    let virtmcu_devenv_img = env::var("VIRTMCU_DEVENV_IMG")
        .unwrap_or_else(|_| format!("{}/{}:{}", registry, devenv_img, image_tag));
    let virtmcu_ci_img = env::var("VIRTMCU_CI_IMG")
        .unwrap_or_else(|_| format!("{}/{}:{}", registry, ci_img, image_tag));
    let virtmcu_ci_asan_img = env::var("VIRTMCU_CI_ASAN_IMG")
        .unwrap_or_else(|_| format!("{}/{}:{}-asan", registry, ci_img, image_tag));

    sh.set_var("VIRTMCU_DEVENV_IMG", &virtmcu_devenv_img);
    sh.set_var("VIRTMCU_CI_IMG", &virtmcu_ci_img);
    sh.set_var("VIRTMCU_CI_ASAN_IMG", &virtmcu_ci_asan_img);

    let use_asan = env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1";
    let use_tsan = env::var("VIRTMCU_USE_TSAN").unwrap_or_default() == "1";

    let build_suffix = if use_asan {
        "-asan"
    } else if use_tsan {
        "-tsan"
    } else {
        ""
    };

    let curdir = env::current_dir()?;
    let qemu_src = env::var("QEMU_SRC")
        .unwrap_or_else(|_| curdir.join("third_party/qemu").display().to_string());
    let qemu_build = env::var("QEMU_BUILD")
        .unwrap_or_else(|_| format!("{}/build-virtmcu{}", qemu_src, build_suffix));

    let jobs = cmd!(
        sh,
        "bash -c 'nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4'"
    )
    .read()
    .unwrap_or_else(|_| "4".to_string());

    let in_container = Path::new("/.dockerenv").exists()
        || Path::new("/run/.containerenv").exists()
        || env::var("USER").unwrap_or_default() == "vscode";
    let uv_run_opts = if in_container {
        vec!["--no-project"]
    } else {
        vec!["--active"]
    };

    // Docker CI run helpers
    let host_uid = cmd!(sh, "id -u")
        .read()
        .unwrap_or_else(|_| "1000".to_string());
    let host_gid = cmd!(sh, "id -g")
        .read()
        .unwrap_or_else(|_| "1000".to_string());

    let run_ci = |sh: &Shell, cmd: &str, img: &str| -> Result<()> {
        let curdir_str = curdir.display().to_string();
        cmd!(sh, "docker run --rm --net=host -v {curdir_str}:/workspace -w /workspace -e HOST_UID={host_uid} -e HOST_GID={host_gid} -e CI=true -e VIRTMCU_STALL_TIMEOUT_MS=120000 -e VIRTMCU_USE_PREBUILT_QEMU=1 {img} {cmd}")
            .run()?;
        Ok(())
    };

    let ensure_image = |sh: &Shell, img: &str, bake_target: &str| -> Result<()> {
        if cmd!(sh, "docker image inspect {img}")
            .quiet()
            .run()
            .is_err()
        {
            println!("==> Image {} not found locally. Pulling...", img);
            if cmd!(sh, "docker pull {img}").run().is_err() {
                println!("==> Pull failed. Building locally...");
                cmd!(sh, "docker buildx bake {bake_target} --load").run()?;
            }
        }
        Ok(())
    };

    match cli.command {
        Commands::Bootstrap => {
            cmd!(sh, "cargo run -p virtmcu-cli -- setup bootstrap").run()?;
        }
        Commands::SyncVersions => {
            println!("==> Synchronizing dependency versions...");
            cmd!(sh, "cargo run -p virtmcu-cli -- setup sync-versions").run()?;
            println!("✓ Versions synchronized.");
        }
        Commands::BuildQemu | Commands::Build => {
            println!("==> Rebuilding QEMU (jobs={})...", jobs);
            if let Ok(xtask_exe) = env::current_exe() {
                let mut exe_str = xtask_exe.display().to_string();
                if exe_str.ends_with(" (deleted)") {
                    exe_str = exe_str.replace(" (deleted)", "");
                }
                sh.set_var("XTASK_BIN", exe_str);
            }
            cmd!(sh, "make -C {qemu_build} -j{jobs}").run()?;
            cmd!(sh, "make -C {qemu_build} install").run()?;
            println!("✓ Done.");
        }
        Commands::BuildRustModules { rust_dir, target_dir, out_dir, artifacts } => {
            env::set_current_dir(&rust_dir)?;

            if cmd!(sh, "command -v lld").quiet().run().is_ok() {
                let current_rustflags = env::var("RUSTFLAGS").unwrap_or_default();
                sh.set_var("RUSTFLAGS", format!("{} -C link-arg=-fuse-ld=lld", current_rustflags));
            }

            let use_asan = env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1";
            let use_tsan = env::var("VIRTMCU_USE_TSAN").unwrap_or_default() == "1";
            let mut build_target = env::var("CARGO_BUILD_TARGET").ok();

            if (use_asan || use_tsan) && build_target.is_none() {
                sh.set_var("RUSTC_BOOTSTRAP", "1");
                let current_rustflags = env::var("RUSTFLAGS").unwrap_or_default();
                if use_asan {
                    sh.set_var("RUSTFLAGS", format!("{} -Zsanitizer=address", current_rustflags));
                    sh.set_var("HOST_CFLAGS", "");
                    sh.set_var("HOST_CXXFLAGS", "");
                } else if use_tsan {
                    sh.set_var("RUSTFLAGS", format!("{} -Z sanitizer=thread", current_rustflags));
                }

                let target = cmd!(sh, "bash -c 'rustc -vV | grep \"host:\" | awk \"{print \\$2}\"'").read()?;
                sh.set_var("CARGO_BUILD_TARGET", &target);
                build_target = Some(target);
            }

            println!("Building Rust workspace in {} with target-dir {}", rust_dir, target_dir);

            let mut final_target_dir = target_dir.clone();
            let fs_type_out = cmd!(sh, "bash -c 'df -T \"{target_dir}\" 2>/dev/null | awk \"NR==2 {print \\$2}\" || df -T \"$(dirname \"{target_dir}\")\" 2>/dev/null | awk \"NR==2 {print \\$2}\" || true'").read().unwrap_or_default();
            let fs_type = fs_type_out.trim();

            if fs_type == "virtiofs" || fs_type == "fakeowner" || fs_type == "9p" {
                let uid = cmd!(sh, "id -u").read().unwrap_or_else(|_| "1000".to_string());
                let safe_target = format!("/tmp/virtmcu-rust-target-{}", uid);
                println!("WARNING: {} is on a {} mount. Redirecting Cargo target-dir to {} to avoid Bus errors.", target_dir, fs_type, safe_target);
                final_target_dir = safe_target;
            }

            fs::create_dir_all(&final_target_dir)?;

            // Disconnect from Ninja's jobserver
            env::remove_var("MAKEFLAGS");

            let num_jobs = cmd!(sh, "bash -c 'nproc 2>/dev/null || sysctl -n hw.logicalcpu 2>/dev/null || echo 4'").read().unwrap_or_else(|_| "4".to_string());

            sh.set_var("CARGO_UNSTABLE_BINDEPS", "true");
            sh.set_var("RUSTC_BOOTSTRAP", "1");

            // Build the full workspace but exclude non-QEMU-plugin tool packages.
            // Using --workspace with --exclude is correct: Cargo handles package name
            // lookup, so we avoid the hyphen/underscore normalisation problem.
            // Excluded packages are coordinator tools and CLI tools that have their own
            // build step (`make build-test-artifacts`) and must not be compiled here
            // because they reference types deleted from virtmcu-wire.
            const EXCLUDED_TOOLS: &[&str] = &[
                "virtmcu-coord",
                "cyber_bridge",
                "virtmcu-physical-node",
                "virtmcu-physics-gateway",
                "virtmcu-resd-replay",
                "gen-pendulum-resd",
                "pendulum-mock-physics",
                "stress_adapter",
                "virtmcu-test-runner",
                "native-integration",
                "virtmcu-cli",
                "xtask",
            ];
            let mut cargo_cmd = cmd!(sh, "cargo build --release --workspace --target-dir {final_target_dir} --jobs {num_jobs}");
            for pkg in EXCLUDED_TOOLS {
                cargo_cmd = cargo_cmd.args(["--exclude", pkg]);
            }
            if let Some(t) = &build_target {
                cargo_cmd = cargo_cmd.args(["--target", t]);
            }
            cargo_cmd.run()?;

            for pair in artifacts {
                let parts: Vec<&str> = pair.split(':').collect();
                if parts.len() != 2 { continue; }
                let lib = parts[1];
                
                let mut src_path = format!("{}/release/{}", final_target_dir, lib);
                if !Path::new(&src_path).exists() {
                    if let Some(t) = &build_target {
                        src_path = format!("{}/{}/release/{}", final_target_dir, t, lib);
                    }
                }

                println!("Copying {} to {}/{}", src_path, out_dir, lib);
                fs::copy(&src_path, format!("{}/{}", out_dir, lib))?;
            }

            println!("Listing outputs in {}:", out_dir);
            // Outputs were successfully copied.
        }
        Commands::GenModuleTrigger { out, obj, extra_inc, extra_c, objs } => {
            if let Some(parent) = Path::new(&out).parent() {
                fs::create_dir_all(parent)?;
            }

            let mut content = String::new();
            content.push_str("#include \"qemu/osdep.h\"\n");
            content.push_str("#include \"qemu/module.h\"\n");

            if let Some(inc) = extra_inc {
                content.push_str(&inc);
                content.push('\n');
            }

            if let Some(o) = obj {
                content.push_str(&format!("module_obj(\"{}\");\n", o));
            }

            for o in objs {
                content.push_str(&format!("module_obj(\"{}\");\n", o));
            }

            if let Some(c) = extra_c {
                content.push_str(&c);
                content.push('\n');
            }

            fs::write(out, content)?;
        }
        Commands::BuildZenohC => {
            let zenohc_build_dir = if use_asan {
                "third_party/zenoh-c-src/build-asan"
            } else {
                "third_party/zenoh-c-src/build-release"
            };
            println!("==> Checking Zenoh-C build...");
            cmd!(sh, "cmake --build {zenohc_build_dir} -j{jobs}").run()?;
            cmd!(sh, "cmake --install {zenohc_build_dir}").run()?;
        }
        Commands::BuildFlatcc => {
            let flatcc_build_dir = "third_party/flatcc-src/build";
            println!("==> Checking FlatCC build...");
            cmd!(
                sh,
                "cmake --build {flatcc_build_dir} -j{jobs} --target install"
            )
            .run()?;
        }
        Commands::BuildThirdParty => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} build-zenoh-c").run()?;
            cmd!(sh, "{xtask} build-flatcc").run()?;
            cmd!(sh, "{xtask} build-qemu").run()?;
        }
        Commands::BuildTestArtifacts => {
            let fixtures = vec![
                "tests/fixtures/guest_apps/boot_arm",
                "tests/fixtures/guest_apps/uart_echo",
                "tests/fixtures/guest_apps/telemetry_wfi",
                "tests/fixtures/guest_apps/actuator",
                "tests/fixtures/guest_apps/boot_riscv",
                "tests/fixtures/guest_apps/flexray_bridge",
                "tests/fixtures/guest_apps/spi_bridge",
                "tests/fixtures/guest_apps/lin_bridge",
                "tests/fixtures/guest_apps/complex_board",
                "tests/fixtures/guest_apps/perf_bench",
            ];
            for f in fixtures {
                cmd!(sh, "make -C {f} -j{jobs}").run()?;
            }
            if env::var("CI").unwrap_or_default() == "true"
                && cmd!(sh, "command -v virtmcu-coord")
                    .quiet()
                    .run()
                    .is_ok()
            {
                println!("==> CI detected: Skipping Rust tools build (using pre-compiled binary in PATH)");
            } else {
                println!("==> Building test tools (virtmcu-coord, cyber_bridge, stress_adapter)...");
                let mut rustflags = String::new();
                let mut bootstrap = "0";
                if use_asan {
                    rustflags.push_str("-Zsanitizer=address ");
                    bootstrap = "1";
                }
                if use_tsan {
                    rustflags.push_str("-Zsanitizer=thread ");
                    bootstrap = "1";
                }

                let triple = cmd!(
                    sh,
                    "bash -c 'rustc -vV | grep host: | awk \"{print \\$2}\"'"
                )
                .read()?;
                let _rustflags = sh.push_env("RUSTFLAGS", rustflags);
                let _rustc_bootstrap = sh.push_env("RUSTC_BOOTSTRAP", bootstrap);
                let _host_cflags = sh.push_env("HOST_CFLAGS", "");
                let _host_cxxflags = sh.push_env("HOST_CXXFLAGS", "");
                let target_dir = format!("target{}", build_suffix);
                let _cargo_target_dir = sh.push_env("CARGO_TARGET_DIR", &target_dir);
                cmd!(sh, "cargo build --release -j{jobs} -p virtmcu-coord -p cyber_bridge -p stress_adapter --target {triple}").run()?;
            }
        }


        Commands::Run { extra_args } => {
            let dtb_arg = if Path::new("tests/fixtures/guest_apps/boot_arm/minimal.dtb").exists() {
                vec!["--dtb", "tests/fixtures/guest_apps/boot_arm/minimal.dtb"]
            } else {
                vec![]
            };
            let kernel_arg = if Path::new("tests/fixtures/guest_apps/boot_arm/hello.elf").exists() {
                vec!["--kernel", "tests/fixtures/guest_apps/boot_arm/hello.elf"]
            } else {
                vec![]
            };

            let mut command = cmd!(sh, "bash target/release/virtmcu-run");
            command = command
                .args(dtb_arg)
                .args(kernel_arg)
                .args(["-nographic", "-m", "128M"])
                .args(extra_args);
            command.run()?;
        }
        Commands::CiCheck => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            run_ci(&sh, "make test-check", &virtmcu_ci_img)?;
        }
        Commands::CiLint => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            run_ci(&sh, "make test-lint", &virtmcu_ci_img)?;
        }
        Commands::CiUnit => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            run_ci(&sh, "make test-unit", &virtmcu_ci_img)?;
        }
        Commands::CiUnitCoverage => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            run_ci(&sh, "make test-unit-coverage", &virtmcu_ci_img)?;
        }
        Commands::CiUnitMiri => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            run_ci(&sh, "make test-unit-miri", &virtmcu_ci_img)?;
        }
        Commands::CiIntegration { domain } => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            let domain_val = domain.unwrap_or_else(|| "all".to_string());
            run_ci(
                &sh,
                &format!("make test-integration DOMAIN={}", domain_val),
                &virtmcu_ci_img,
            )?;
        }
        Commands::CiIntegrationCoverage => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            run_ci(&sh, "make test-integration-coverage", &virtmcu_ci_img)?;
        }
        Commands::CiIntegrationAsan { domain } => {
            ensure_image(&sh, &virtmcu_ci_asan_img, "ci-asan")?;
            let domain_val = domain.unwrap_or_else(|| "all".to_string());
            run_ci(
                &sh,
                &format!("make test-integration-asan DOMAIN={}", domain_val),
                &virtmcu_ci_asan_img,
            )?;
            println!("\n✓ ci-integration-asan passed.");
        }
        Commands::CiPeripheralCoverage => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;
            run_ci(&sh, "make test-peripheral-coverage", &virtmcu_ci_img)?;
        }
        Commands::CiBuildThirdParty => {
            cmd!(sh, "cargo xtask third-party-builder").run()?;
        }
        Commands::CiBuildThirdPartyAsan => {
            let _env = sh.push_env("VIRTMCU_USE_ASAN", "1");
            cmd!(sh, "cargo xtask third-party-builder").run()?;
        }
        Commands::CiFull => {
            ensure_image(&sh, &virtmcu_ci_img, "ci")?;

            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} ci-lint").run()?;
            cmd!(sh, "{xtask} ci-unit").run()?;
            cmd!(sh, "{xtask} ci-integration-asan").run()?;
            cmd!(sh, "{xtask} ci-unit-miri").run()?;

            fs::create_dir_all("coverage-data")?;
            let curdir_str = curdir.display().to_string();
            cmd!(sh, "docker run --rm --net=host -v {curdir_str}:/workspace -w /workspace -e HOST_UID={host_uid} -e HOST_GID={host_gid} -e CI=true -e VIRTMCU_STALL_TIMEOUT_MS=120000 -e VIRTMCU_USE_PREBUILT_QEMU=1 -e GCOV_PREFIX=/workspace/coverage-data -e GCOV_PREFIX_STRIP=3 {virtmcu_ci_img} make test-integration DOMAIN=all").run()?;

            cmd!(sh, "{xtask} ci-integration-coverage").run()?;
            cmd!(sh, "{xtask} ci-peripheral-coverage").run()?;
            println!("\n✓ ci-full passed.");
        }
        Commands::SetupDev => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} bootstrap").run()?;
            cmd!(sh, "{xtask} sync-versions").run()?;
            cmd!(sh, "{xtask} build-qemu").run()?;
        }
        Commands::DevAll => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} build-qemu").run()?;
            cmd!(sh, "{xtask} build-test-artifacts").run()?;
            cmd!(sh, "{xtask} test-check").run()?;
            cmd!(sh, "{xtask} test-integration").run()?;
            cmd!(sh, "{xtask} test-peripheral-coverage").run()?;
        }
        Commands::DevCheck => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} test-lint").run()?;
            cmd!(sh, "{xtask} test-unit").run()?;
            cmd!(sh, "{xtask} test-unit-coverage").run()?;
        }
        Commands::DevLint => {
            cmd!(sh, "cargo run -p virtmcu-test-runner --release -- lint").run()?;
        }
        Commands::DevUnit => {
            cmd!(
                sh,
                "cargo run -p virtmcu-test-runner --release -- run --tier unit"
            )
            .run()?;
        }
        Commands::DevUnitCoverage => {
            cmd!(sh, "cargo run -p virtmcu-test-runner --release -- coverage").run()?;
        }
        Commands::DevUnitMiri => {
            cmd!(sh, "cargo run -p virtmcu-test-runner --release -- miri").run()?;
        }
        Commands::DevIntegration { domain } => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} build-test-artifacts").run()?;
            
            let mut runner = cmd!(
                sh,
                "cargo run -p virtmcu-test-runner --release -- run --tier integration"
            );
            if let Some(d) = domain {
                runner = runner.args(["--domain", &d]);
            }
            runner.run()?;
        }
        Commands::DevIntegrationCoverage => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} build-test-artifacts").run()?;
            
            cmd!(
                sh,
                "cargo run -p virtmcu-test-runner --release -- coverage --integration"
            )
            .run()?;
        }
        Commands::DevIntegrationAsan { domain } => {
            let xtask = env::current_exe()?;
            let _env = sh.push_env("VIRTMCU_USE_ASAN", "1");
            cmd!(sh, "{xtask} build-test-artifacts").run()?;
            
            let mut runner = cmd!(
                sh,
                "cargo run -p virtmcu-test-runner --release -- run --tier integration --asan"
            );
            if let Some(d) = domain {
                runner = runner.args(["--domain", &d]);
            }
            runner.run()?;
        }
        Commands::DevPeripheralCoverage => {
            cmd!(
                sh,
                "cargo run -p virtmcu-test-runner --release -- coverage --peripheral"
            )
            .run()?;
        }
        Commands::InstallGitHooks => {
            println!("==> Installing Git hooks...");
            fs::create_dir_all(".git/hooks")?;
            fs::write(
                ".git/hooks/pre-commit",
                "#!/bin/sh\nset -e\ncargo xtask test-lint\n",
            )?;
            fs::write(
                ".git/hooks/pre-push",
                "#!/bin/sh\nset -e\ncargo xtask test-unit\n",
            )?;
            cmd!(sh, "chmod +x .git/hooks/pre-push .git/hooks/pre-commit").run()?;
            println!("✓ Git hooks installed: pre-commit (lint) and pre-push (unit).");
        }
        Commands::FmtAll => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} fmt-python").run()?;
            cmd!(sh, "{xtask} fmt-rust").run()?;
            cmd!(sh, "{xtask} fmt-meson").run()?;
            cmd!(sh, "{xtask} fmt-c").run()?;
            cmd!(sh, "{xtask} fmt-yaml").run()?;
        }
        Commands::FmtPython => {
            println!("==> ruff format + fix...");
            cmd!(sh, "uv run")
                .args(&uv_run_opts)
                .args(["ruff", "format", "."])
                .run()?;
            cmd!(sh, "uv run")
                .args(&uv_run_opts)
                .args(["ruff", "check", ".", "--fix"])
                .run()?;
        }
        Commands::FmtRust => {
            println!("==> cargo fmt...");
            cmd!(sh, "cargo fmt --all").run()?;
        }
        Commands::FmtMeson => {
            println!("==> meson format...");
            cmd!(sh, "meson fmt -i hw/meson.build")
                .run()
                .context("meson format failed")?;
            println!("✓ meson format passed.");
        }
        Commands::FmtC => {
            println!("==> clang-format...");
            cmd!(sh, "bash -c 'find hw tools tests -type f \\( -name \"*.c\" -o -name \"*.h\" -o -name \"*.cpp\" -o -name \"*.cc\" -o -name \"*.hpp\" \\) -not -path \"*/rust/*\" -not -path \"*/third_party/*\" -print0 | xargs -0 clang-format -i'").run().context("clang-format failed")?;
            println!("✓ clang-format passed.");
        }
        Commands::FmtYaml => {
            println!("==> stripping trailing whitespace from YAMLs...");
            cmd!(sh, "bash -c 'find . -type f \\( -name \"*.yml\" -o -name \"*.yaml\" \\) -not -path \"*/third_party/*\" -print0 | xargs -0 sed -i \"s/[[:space:]]*$$//\"'").run()?;
        }
        Commands::DockerDev => {
            let xtask = env::current_exe()?;
            cmd!(sh, "docker buildx bake base --load").run()?;
            cmd!(sh, "{xtask} smoke-base").run()?;
            cmd!(sh, "docker buildx bake toolchain --load").run()?;
            cmd!(sh, "{xtask} smoke-toolchain").run()?;
            cmd!(sh, "docker buildx bake devenv --load").run()?;
            cmd!(sh, "{xtask} smoke-devenv").run()?;
            println!("✓ All dev-base stages built and verified.");
        }
        Commands::DockerAll => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} docker-dev").run()?;
            cmd!(sh, "docker buildx bake third-party-base --load").run()?;
            cmd!(sh, "docker buildx bake ci --load").run()?;
            cmd!(sh, "{xtask} smoke-ci").run()?;
            cmd!(sh, "docker buildx bake ci-asan --load").run()?;
            cmd!(sh, "{xtask} smoke-ci-asan").run()?;
            cmd!(sh, "docker buildx bake runtime --load").run()?;
        }
        Commands::DockerBase => {
            cmd!(sh, "docker buildx bake base --load").run()?;
        }
        Commands::DockerToolchain => {
            cmd!(sh, "docker buildx bake toolchain --load").run()?;
        }
        Commands::DockerDevenv => {
            cmd!(sh, "docker buildx bake devenv --load").run()?;
        }
        Commands::ThirdPartyBuilder => {
            if use_asan {
                cmd!(sh, "docker buildx bake third-party-base-asan --load").run()?;
            } else {
                cmd!(sh, "docker buildx bake third-party-base --load").run()?;
            }
        }
        Commands::DockerCi => {
            cmd!(sh, "docker buildx bake ci --load").run()?;
        }
        Commands::DockerCiAsan => {
            cmd!(sh, "docker buildx bake ci-asan --load").run()?;
        }
        Commands::DockerRuntime => {
            cmd!(sh, "docker buildx bake runtime --load").run()?;
        }
        Commands::SmokeBase => {
            println!("==> Smoke test: base");
            let img = format!("{}/base:{}-{}", registry, image_tag, arch);
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'id vscode'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'sudo -n true'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'zsh --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'test -d /home/vscode/.oh-my-zsh'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'locale | grep \"LANG=en_US.UTF-8\"'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'gh --version | head -1'").run()?;
            println!("✓ base smoke test passed");
        }
        Commands::SmokeToolchain => {
            println!("==> Smoke test: toolchain");
            let img = format!("{}/toolchain:{}-{}", registry, image_tag, arch);
            let python_version = sh.var("PYTHON_VERSION").unwrap_or_else(|_| "3.13.1".to_string());
            let py_check = format!("uv run --python {} python --version", python_version);
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'arm-none-eabi-gcc --version | head -1'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'riscv64-linux-gnu-gcc --version | head -1'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c {py_check}").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'cmake --version | head -1'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'flatc --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {img} bash -c 'meson --version'").run()?;
            println!("✓ toolchain smoke test passed");
        }
        Commands::SmokeDevenv => {
            println!("==> Smoke test: devenv");
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'node --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'npm --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'cargo --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'cargo tarpaulin --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'rustc --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'mdbook --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'mdbook-mermaid --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'which mdbook-pdf'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'chromium --version'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'arm-none-eabi-gcc --version | head -1'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_devenv_img} bash -c 'uv --version'").run()?;
            println!("✓ devenv smoke test passed");
        }
        Commands::SmokeCi => {
            println!("==> Smoke test: ci");
            cmd!(sh, "docker run --rm --net=host {virtmcu_ci_img} qemu-system-arm --version").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_ci_img} bash -c 'qemu-system-riscv32 --version | head -1'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_ci_img} bash -c 'qemu-system-riscv64 --version | head -1'").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_ci_img} bash -c 'ls ${{QEMU_MODULE_DIR}}/*.so | head -5'").run()?;
            println!("✓ ci smoke test passed");
        }
        Commands::SmokeCiAsan => {
            println!("==> Smoke test: ci-asan");
            cmd!(sh, "docker run --rm --net=host {virtmcu_ci_asan_img} qemu-system-arm --version").run()?;
            cmd!(sh, "docker run --rm --net=host {virtmcu_ci_asan_img} bash -c 'nm $(which qemu-system-arm) | grep -q __asan'").run()?;
            println!("✓ ci-asan smoke test passed");
        }
        Commands::CleanSim => {
            cmd!(sh, "cargo run -p virtmcu-cli -- setup cleanup-sim").run()?;
        }
        Commands::DeleteProfraw => {
            println!("==> Deleting backup and profile raw files...");
            cmd!(
                sh,
                "bash -c 'find . -type f \\( -name \"*~\" -o -name \"*profraw\" \\) -delete'"
            )
            .run()?;
        }
        Commands::CleanDebug | Commands::Clean => {
            println!("==> Cleaning generated files and test artifacts...");
            cmd!(sh, "bash -c 'find . -name \"*.pyc\" -delete'").run()?;
            cmd!(sh, "bash -c 'find . -name \"__pycache__\" -type d -exec rm -rf {{}} + 2>/dev/null || true'").run()?;
            cmd!(sh, "bash -c 'find . -name \"*.profraw\" -delete'").run()?;
            cmd!(sh, "bash -c 'find . -name \"*.log\" -delete'").run()?;
            cmd!(
                sh,
                "bash -c 'find . -name \"*.dtb\" -not -path \"./third_party/*\" -delete'"
            )
            .run()?;
            cmd!(
                sh,
                "bash -c 'find . -name \"*.o\" -not -path \"./third_party/*\" -delete'"
            )
            .run()?;
            cmd!(sh, "bash -c 'find . -name \"*.elf\" -not -path \"./tests/firmware/*\" -not -path \"./third_party/*\" -delete'").run()?;
            cmd!(sh, "bash -c 'find . -name \"*.cli\" -delete'").run()?;
            cmd!(sh, "bash -c 'find . -name \"*.arch\" -delete'").run()?;
            cmd!(sh, "bash -c 'find . -name \"*.gcov\" -delete'").run()?;
            cmd!(sh, "bash -c 'find . -name \"*.gcda\" -delete'").run()?;
            cmd!(
                sh,
                "bash -c 'find . -name \"*.gcno\" -not -path \"./third_party/*\" -delete'"
            )
            .run()?;
            cmd!(sh, "bash -c 'find . -name \"virtmcu-timeout-*\" -delete'").run()?;
            cmd!(sh, "bash -c 'find . -name \"qmp-timeout-*\" -delete'").run()?;
            cmd!(sh, "rm -f .coverage").run()?;
            cmd!(sh, "rm -rf .pytest_cache .ruff_cache .hypothesis test-results/ tests/fixtures/guest_apps/*/results/ install/").run()?;
            cmd!(sh, "rm -f *_output.txt log.html report.html output.xml").run()?;
            cmd!(sh, "rm -rf tools/cyber_bridge/target tools/systemc_adapter/build tools/virtmcu-coord/target hw/rust/target").run()?;
            cmd!(
                sh,
                "rm -rf {qemu_src}/build-virtmcu/install {qemu_src}/build-virtmcu-asan/install"
            )
            .run()?;
            println!("✓ Clean complete (QEMU sources remain).");
        }
        Commands::Distclean => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} clean").run()?;
            cmd!(sh, "rm -rf third_party test-results").run()?;
            cmd!(sh, "bash -c 'rm -rf .ci-target*'").run()?;
            println!(
                "✓ Deep clean complete. Run 'cargo xtask bootstrap' to rebuild the environment."
            );
        }
        Commands::Book => {
            println!("==> Building mdBook (HTML + PDF)...");
            if cmd!(sh, "command -v mdbook").quiet().run().is_ok() {
                if cmd!(sh, "command -v mdbook-mermaid").quiet().run().is_err() {
                    println!("❌ mdbook-mermaid not installed.");
                    std::process::exit(1);
                }
                if cmd!(sh, "command -v mdbook-pdf").quiet().run().is_err() {
                    println!("❌ mdbook-pdf not installed.");
                    std::process::exit(1);
                }
                cmd!(sh, "mdbook build").run()?;
            } else {
                println!("❌ mdbook not installed. Please restart devcontainer or run: cargo install mdbook");
                std::process::exit(1);
            }
            println!("✓ mdBook built in target/book (HTML and PDF).");
            cmd!(
                sh,
                "mv target/book/pdf/output.pdf target/book/pdf/virtmcu_book.pdf"
            )
            .run()?;
        }
        Commands::BookServe => {
            let xtask = env::current_exe()?;
            cmd!(sh, "{xtask} book").run()?;
            println!("==> Serving mdBook...");
            println!("    Click this link to open: http://localhost:8080");
            cmd!(sh, "python3 -m http.server -d target/book 8080").run()?;
        }
        Commands::Tag { version } => {
            if !version.starts_with('v') || version.split('.').count() != 3 {
                println!(
                    "❌ VERSION must match vMAJOR.MINOR.PATCH (got: {})",
                    version
                );
                std::process::exit(1);
            }
            if let Ok(status) = cmd!(sh, "git status --porcelain").read() {
                if !status.is_empty() {
                    println!("❌ Working tree is dirty — commit or stash changes before releasing");
                    std::process::exit(1);
                }
            }
            if cmd!(
                sh,
                "bash -c 'test \"$(git rev-parse --abbrev-ref HEAD)\" = \"main\"'"
            )
            .run()
            .is_err()
            {
                println!("❌ Releases must be tagged from the main branch");
                std::process::exit(1);
            }
            if cmd!(sh, "git rev-parse {version}").quiet().run().is_ok() {
                println!("❌ Tag {} already exists", version);
                std::process::exit(1);
            }
            let bare_version = version.trim_start_matches('v');
            fs::write("VERSION", format!("{}\n", bare_version))?;
            cmd!(sh, "git add VERSION").run()?;
            cmd!(sh, "git commit -m 'chore: release {version}'").run()?;
            cmd!(sh, "git tag -a {version} -m 'Release {version}'").run()?;
            cmd!(sh, "git push origin main {version}").run()?;
            println!("✓ Tagged and pushed {}", version);
            println!(
                "  CI will publish versioned images and create a GitHub Release automatically."
            );
        }
    }

    Ok(())
}

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use tracing::{error, info, warn};

fn get_build_deps() -> HashMap<String, String> {
    let mut versions = HashMap::new();
    if let Ok(content) = std::fs::read_to_string("BUILD_DEPS") {
        for line in content.lines() {
            if line.contains('=') && !line.starts_with('#') {
                let parts: Vec<&str> = line.splitn(2, '=').collect();
                if parts.len() == 2 {
                    versions.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
                }
            }
        }
    }
    versions
}

pub async fn run_sync_versions() -> Result<()> {
    info!("==> Synchronizing dependency versions natively...");
    let versions = get_build_deps();

    if let Some(zenoh_ver) = versions.get("ZENOH_VERSION") {
        for cargo_path in ["tools/deterministic_coordinator/Cargo.toml", "Cargo.toml"] {
            if let Ok(content) = std::fs::read_to_string(cargo_path) {
                let re = regex::Regex::new(r#"zenoh = "[^"]+""#).unwrap();
                let new_content = re.replace_all(&content, format!(r#"zenoh = "{}""#, zenoh_ver));
                std::fs::write(cargo_path, new_content.to_string())?;
                info!("Updated {} to zenoh {}", cargo_path, zenoh_ver);
            }
        }
    }

    Ok(())
}

pub async fn run_bootstrap() -> Result<()> {
    info!("==> Running bootstrap...");

    // 1. Submodule update
    info!("  -> Updating submodules...");
    Command::new("git")
        .args(["submodule", "update", "--init", "--recursive"])
        .status()?;

    let workspace_root = std::env::current_dir()?;
    let third_party = workspace_root.join("third_party");
    std::fs::create_dir_all(&third_party)?;

    let deps = get_build_deps();

    // 2. Ensure Zenoh-C
    let zenoh_c_dir = third_party.join("zenoh-c-src");
    if !zenoh_c_dir.exists() {
        let zenoh_ver = deps
            .get("ZENOH_VERSION")
            .map(|s| s.as_str())
            .unwrap_or("main");
        info!("  -> Cloning Zenoh-C ({})", zenoh_ver);
        Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "--branch",
                zenoh_ver,
                "https://github.com/eclipse-zenoh/zenoh-c.git",
                zenoh_c_dir.to_str().unwrap(),
            ])
            .status()?;
    }
    configure_zenoh_c(&zenoh_c_dir).await?;

    // 3. Ensure FlatCC
    let flatcc_dir = third_party.join("flatcc-src");
    if !flatcc_dir.exists() {
        let flatcc_ver = deps
            .get("FLATCC_VERSION")
            .map(|s| s.as_str())
            .unwrap_or("0.6.1");
        let branch = if flatcc_ver.starts_with('v') {
            flatcc_ver.to_string()
        } else {
            format!("v{}", flatcc_ver)
        };
        info!("  -> Cloning FlatCC ({})", branch);
        Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "--branch",
                &branch,
                "https://github.com/dvidelabs/flatcc.git",
                flatcc_dir.to_str().unwrap(),
            ])
            .status()?;
    }
    configure_flatcc(&flatcc_dir).await?;

    // 4. Patch QEMU
    let qemu_dir = third_party.join("qemu");
    if qemu_dir.exists() {
        run_patch_qemu(&qemu_dir).await?;
    } else {
        warn!("QEMU source not found in third_party/qemu. Skipping patches.");
    }

    // 5. Configure QEMU
    if qemu_dir.exists() {
        configure_qemu(&qemu_dir).await?;
    }

    Ok(())
}

async fn configure_zenoh_c(path: &Path) -> Result<()> {
    let use_asan = std::env::var("VIRTMCU_USE_ASAN").unwrap_or_else(|_| "0".to_string()) == "1";
    let build_dir = if use_asan {
        path.join("build-asan")
    } else {
        path.join("build-release")
    };

    if build_dir.join("CMakeCache.txt").exists() {
        return Ok(());
    }

    info!("  -> Configuring Zenoh-C in {}...", build_dir.display());
    std::fs::create_dir_all(&build_dir)?;

    let mut args = vec![
        "-S".to_string(),
        path.to_str().unwrap().to_string(),
        "-B".to_string(),
        build_dir.to_str().unwrap().to_string(),
        "-DCMAKE_BUILD_TYPE=Release".to_string(),
        "-DZENOHC_BUILD_WITH_SHARED_MEMORY=OFF".to_string(),
    ];

    if use_asan {
        args.push(
            "-DCMAKE_SHARED_LINKER_FLAGS=-fsanitize=address -fsanitize=undefined".to_string(),
        );
        args.push("-DCMAKE_EXE_LINKER_FLAGS=-fsanitize=address -fsanitize=undefined".to_string());
    }

    let status = Command::new("cmake").args(&args).status()?;
    if !status.success() {
        return Err(anyhow!("Zenoh-C configuration failed"));
    }
    Ok(())
}

async fn configure_flatcc(path: &Path) -> Result<()> {
    let build_dir = path.join("build");
    if build_dir.join("CMakeCache.txt").exists() {
        return Ok(());
    }

    info!("  -> Configuring FlatCC in {}...", build_dir.display());
    std::fs::create_dir_all(&build_dir)?;

    let status = Command::new("cmake")
        .args([
            "-S",
            path.to_str().unwrap(),
            "-B",
            build_dir.to_str().unwrap(),
            "-DFLATCC_INSTALL=ON",
            "-DCMAKE_BUILD_TYPE=Release",
            "-DCMAKE_POLICY_VERSION_MINIMUM=3.5",
        ])
        .status()?;
    if !status.success() {
        return Err(anyhow!("FlatCC configuration failed"));
    }
    Ok(())
}

async fn configure_qemu(qemu_dir: &Path) -> Result<()> {
    let use_asan = std::env::var("VIRTMCU_USE_ASAN").unwrap_or_else(|_| "0".to_string()) == "1";
    let use_tsan = std::env::var("VIRTMCU_USE_TSAN").unwrap_or_else(|_| "0".to_string()) == "1";

    let build_suffix = if use_asan {
        "-asan"
    } else if use_tsan {
        "-tsan"
    } else {
        ""
    };

    let build_dir_name = format!("build-virtmcu{}", build_suffix);
    let build_dir = qemu_dir.join(&build_dir_name);

    if build_dir.join("config-host.mak").exists() {
        info!("QEMU already configured in {}", build_dir.display());
        return Ok(());
    }

    std::fs::create_dir_all(&build_dir)?;

    let arch = std::env::consts::ARCH;
    let qemu_host_arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => arch,
    };

    let target_list = format!("{}-softmmu,riscv32-softmmu,riscv64-softmmu", qemu_host_arch);

    info!("==> Configuring QEMU in {}...", build_dir.display());

    let mut args = vec![
        format!("--prefix={}/install", build_dir.to_str().unwrap()),
        format!("--target-list={}", target_list),
        "--enable-rust".to_string(),
        "--enable-modules".to_string(),
        "--enable-fdt".to_string(),
        "--enable-plugins".to_string(),
        "--enable-debug".to_string(),
        "--enable-gcov".to_string(),
        "--disable-werror".to_string(),
        "--disable-docs".to_string(),
        "--disable-dbus-display".to_string(),
        "--extra-ldflags=-fuse-ld=lld -rdynamic".to_string(),
    ];

    if use_asan {
        args.push("--enable-asan".to_string());
        args.push("--enable-ubsan".to_string());
        args.push("-Db_sanitize=address,undefined".to_string());
    }

    let status = Command::new("../configure")
        .current_dir(&build_dir)
        .args(&args)
        .status()?;

    if !status.success() {
        return Err(anyhow!(
            "QEMU configuration failed in {}",
            build_dir.display()
        ));
    }

    Ok(())
}

pub async fn run_cleanup_sim() -> Result<()> {
    info!("==> Cleaning up simulation processes...");
    // Basic process kill logic
    let processes = [
        "qemu-system-arm",
        "qemu-system-aarch64",
        "qemu-system-riscv64",
        "qemu-system-riscv32",
        "zenohd",
        "zenoh_router",
        "deterministic_coordinator",
        "mmio-socket-bridge",
    ];

    for proc in processes {
        Command::new("pkill").args(["-f", proc]).status().ok();
    }
    Ok(())
}

pub async fn run_generate_schemas() -> Result<()> {
    info!("==> Generating schemas natively...");
    let workspace_root = std::env::current_dir()?;

    // 1. TypeSpec compilation
    info!("  -> Compiling TypeSpec...");
    Command::new("npx")
        .current_dir(workspace_root.join("schema"))
        .args(["tsp", "compile", "world/main.tsp", "--output-dir", "./dist"])
        .status()?;
    std::fs::copy(
        workspace_root.join("schema/dist/@typespec/json-schema/virtmcu_world.schema.json"),
        workspace_root.join("schema/world_schema.json"),
    )?;

    // 2. Fix JSON schema references (Rust native)
    info!("  -> Fixing JSON Schema References...");
    let schema_path = workspace_root.join("schema/world_schema.json");
    let content = std::fs::read_to_string(&schema_path)?;
    let mut schema: serde_json::Value = serde_json::from_str(&content)?;
    fix_json_refs(&mut schema);
    std::fs::write(&schema_path, serde_json::to_string_pretty(&schema)?)?;

    // 3. Generate Rust Models
    info!("  -> Generating Rust Models...");
    Command::new("cargo")
        .current_dir(workspace_root.join("schema/rust_gen"))
        .arg("run")
        .status()?;

    Command::new("rustfmt")
        .current_dir(&workspace_root)
        .args([
            "--edition",
            "2021",
            "tools/deterministic_coordinator/src/generated/topology.rs",
        ])
        .status()?;

    info!("✓ Code generation pipeline completed successfully!");
    Ok(())
}

fn fix_json_refs(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(ref_str)) = map.get("$ref") {
                if !ref_str.starts_with('#') {
                    let new_ref = ref_str.replace(".yaml", "").replace(".json", "");
                    map.insert(
                        "$ref".to_string(),
                        serde_json::Value::String(format!("#/$defs/{}", new_ref)),
                    );
                }
            }
            for val in map.values_mut() {
                fix_json_refs(val);
            }
        }
        serde_json::Value::Array(arr) => {
            for val in arr.iter_mut() {
                fix_json_refs(val);
            }
        }
        _ => {}
    }
}

pub async fn run_check_schemas() -> Result<()> {
    info!("==> Verifying schema generation is up-to-date...");
    run_generate_schemas().await?;

    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()?;
    let status_out = String::from_utf8_lossy(&output.stdout);

    let mut out_of_sync = false;
    for file in [
        "schema/world_schema.json",
        "tools/deterministic_coordinator/src/generated/topology.rs",
    ] {
        if status_out.contains(file) {
            error!(
                "❌ Error: {} is out of date. Please commit the newly generated changes.",
                file
            );
            out_of_sync = true;
        }
    }

    if out_of_sync {
        return Err(anyhow!("Schema artifacts are not synchronized."));
    }

    info!("✅ Generated schema artifacts are perfectly synchronized.");
    Ok(())
}

pub async fn run_patch_qemu(qemu_dir: &Path) -> Result<()> {
    let patched_marker = qemu_dir.join(".virtmcu_patched");
    if patched_marker.exists() {
        info!(
            "  -> QEMU already patched at {}. Skipping.",
            qemu_dir.display()
        );
        return Ok(());
    }

    info!(
        "==> Applying virtmcu patches to QEMU at {}",
        qemu_dir.display()
    );

    let workspace_root = std::env::current_dir()?;

    // 1. Ensure clean state
    if qemu_dir.join(".git").exists() {
        info!("  -> Cleaning QEMU repository...");
        let _ = Command::new("git")
            .current_dir(qemu_dir)
            .args(["am", "--abort"])
            .status();

        let deps = get_build_deps();
        let qemu_ver = deps
            .get("QEMU_VERSION")
            .map(|s| s.as_str())
            .unwrap_or("v11.0.0");
        let tag = if qemu_ver.starts_with('v') {
            qemu_ver.to_string()
        } else {
            format!("v{}", qemu_ver)
        };

        Command::new("git")
            .current_dir(qemu_dir)
            .args(["checkout", "-f", &tag])
            .status()?;
        Command::new("git")
            .current_dir(qemu_dir)
            .args(["reset", "--hard", &tag])
            .status()?;
        Command::new("git")
            .current_dir(qemu_dir)
            .args(["clean", "-fd"])
            .status()?;

        Command::new("git")
            .current_dir(qemu_dir)
            .args(["config", "user.email", "virtmcu-build@example.com"])
            .status()?;
        Command::new("git")
            .current_dir(qemu_dir)
            .args(["config", "user.name", "virtmcu"])
            .status()?;
    }

    // 2. Apply patch series
    let mbx = workspace_root.join("patches/arm-generic-fdt-v3.mbx");
    if mbx.exists() {
        info!("  -> Applying arm-generic-fdt-v3.mbx...");
        let status = Command::new("git")
            .current_dir(qemu_dir)
            .args(["am", "--3way"])
            .arg(&mbx)
            .status()?;
        if !status.success() {
            return Err(anyhow!(
                "Failed to apply arm-generic-fdt-v3.mbx. Manual intervention required in {}.",
                qemu_dir.display()
            ));
        }
    }

    // 3. Simple File manipulations (replacing sed)
    patch_c_file(
        &qemu_dir.join("hw/arm/arm_generic_fdt.c"),
        "mc->minimum_page_bits = 12;",
        "mc->minimum_page_bits = 12;\n\n    /* virtmcu: allow all SysBus devices via -device; arm-generic-fdt loads devices from DTB at runtime */\n    machine_class_allow_dynamic_sysbus_dev(mc, \"sys-bus-device\");",
    )?;

    // 4. Meson subdir
    let hw_meson = qemu_dir.join("hw/meson.build");
    let content = std::fs::read_to_string(&hw_meson)?;
    if !content.contains("subdir('virtmcu')") {
        info!("  -> Injecting subdir('virtmcu') into hw/meson.build...");
        std::fs::write(&hw_meson, format!("{}\nsubdir('virtmcu')\n", content))?;
    }

    // 5. Run Python scripts for now (until they are migrated)
    // Actually, I should migrate them too.
    // But for a first step, I can call them.
    // Wait, the goal is ERADICATION.

    // Migrated logic
    apply_zenoh_hooks(qemu_dir)?;
    apply_zenoh_qapi(qemu_dir)?;
    apply_zenoh_netdev(qemu_dir)?;
    apply_zenoh_chardev(qemu_dir)?;
    apply_fdt_generic_util_fix(qemu_dir)?;
    apply_sysbus_asan_fix(qemu_dir)?;
    apply_rust_asan_fix(qemu_dir)?;

    std::fs::write(patched_marker, "")?;
    info!("✓ QEMU patching completed (Rust-powered).");
    Ok(())
}
fn patch_c_file(path: &Path, marker: &str, replacement: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(path)?;
    if content.contains("virtmcu-patch") || content.contains(replacement) {
        return Ok(());
    }

    if let Some(pos) = content.find(marker) {
        let mut new_content = content.clone();
        new_content.replace_range(pos..pos + marker.len(), replacement);
        std::fs::write(path, new_content)?;
        info!("  patched {}", path.display());
    } else {
        warn!("  marker not found in {}", path.display());
    }
    Ok(())
}

fn apply_zenoh_hooks(qemu: &Path) -> Result<()> {
    info!("  -> Injecting Zenoh hooks...");

    // 1. Create include/virtmcu/hooks.h
    let hooks_h = qemu.join("include/virtmcu/hooks.h");
    let hooks_h_content = r#"/* Generated by virtmcu-cli */
#ifndef VIRTMCU_HOOKS_H
#define VIRTMCU_HOOKS_H

#include "hw/core/cpu.h"
#include "net/net.h"

#if defined(_WIN32)
#define VIRTMCU_EXPORT __declspec(dllexport)
#else
#define VIRTMCU_EXPORT __attribute__((visibility("default")))
#endif

typedef struct {
    int64_t quantum_start_vtime_ns;
    int64_t quantum_delta_ns;
    int64_t absolute_vtime_ns;
} VirtmcuQuantumTiming;

VIRTMCU_EXPORT extern void (*virtmcu_tcg_quantum_hook)(CPUState *cpu);
VIRTMCU_EXPORT extern int (*virtmcu_netdev_hook)(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp);
VIRTMCU_EXPORT extern void (*virtmcu_irq_hook)(void *opaque, int n, int level);
VIRTMCU_EXPORT extern void (*virtmcu_cpu_halt_hook)(CPUState *cpu, bool halted);

VIRTMCU_EXPORT void virtmcu_cpu_set_tcg_hook(void (*cb)(CPUState *cpu));
VIRTMCU_EXPORT void virtmcu_cpu_set_halt_hook(void (*cb)(CPUState *cpu, bool halted));
VIRTMCU_EXPORT void virtmcu_set_irq_hook(void (*cb)(void *opaque, int n, int level));

VIRTMCU_EXPORT void virtmcu_kick_first_cpu_for_quantum(void);
VIRTMCU_EXPORT bool virtmcu_vcpu_should_yield(void);
VIRTMCU_EXPORT bool virtmcu_is_bql_locked(void);
VIRTMCU_EXPORT void virtmcu_safe_bql_unlock(void);
VIRTMCU_EXPORT void virtmcu_safe_bql_lock(void);
VIRTMCU_EXPORT void virtmcu_safe_bql_force_unlock(void);
VIRTMCU_EXPORT void virtmcu_safe_bql_force_lock(void);
VIRTMCU_EXPORT void virtmcu_timer_kick(QEMUTimer *timer);

VIRTMCU_EXPORT extern void (*virtmcu_get_quantum_timing)(VirtmcuQuantumTiming *timing);

#endif
"#;
    std::fs::create_dir_all(hooks_h.parent().unwrap())?;
    std::fs::write(&hooks_h, hooks_h_content)?;

    // 2. Patch accel/tcg/cpu-exec.c
    patch_c_file(
        &qemu.join("accel/tcg/cpu-exec.c"),
        "#include \"internal-common.h\"",
        "#include \"internal-common.h\"\n#include \"virtmcu/hooks.h\"",
    )?;

    patch_c_file(
        &qemu.join("accel/tcg/cpu-exec.c"),
        "while (!cpu_handle_interrupt(cpu, &last_tb)) {",
        "while (!cpu_handle_interrupt(cpu, &last_tb)) {\n        if (virtmcu_tcg_quantum_hook) { virtmcu_tcg_quantum_hook(cpu); }",
    )?;

    // 3. Patch system/cpus.c
    patch_c_file(
        &qemu.join("system/cpus.c"),
        "#include \"qemu/osdep.h\"",
        "#include \"qemu/osdep.h\"\n#include \"virtmcu/hooks.h\"",
    )?;

    Ok(())
}

fn apply_zenoh_qapi(qemu: &Path) -> Result<()> {
    info!("  -> Injecting Zenoh QAPI extensions...");
    let net_json = qemu.join("qapi/net.json");
    let char_json = qemu.join("qapi/char.json");
    let qom_json = qemu.join("qapi/qom.json");

    // 1. net.json
    if net_json.exists() {
        patch_c_file(
            &net_json,
            "# @vhost-vdpa: since 5.1",
            "# @vhost-vdpa: since 5.1\n#\n# @virtmcu: since 11.0",
        )?;
        patch_c_file(
            &net_json,
            "'vhost-vdpa',",
            "'vhost-vdpa',\n            'virtmcu',",
        )?;

        let netdev_struct = r#"
##
# @NetdevVirtmcuOptions:
#
# virtmcu: Virtual clock network backend
#
# @node: The node ID
# @transport: The transport to use (zenoh or unix) (optional)
# @router: The zenoh router address (optional)
# @topic: The topic to publish/subscribe to (optional)
# @max-backlog: Maximum number of frames in the RX backlog
#     (default: 256) (optional)
#
# Since: 11.0
##
{ 'struct': 'NetdevVirtmcuOptions',
  'data': {
    'node': 'str',
    '*transport': 'str',
    '*router': 'str',
    '*topic': 'str',
    '*max-backlog': 'size' } }

##
# @NetdevVmnetHostOptions:"#;
        let content = std::fs::read_to_string(&net_json)?;
        if !content.contains("NetdevVirtmcuOptions") {
            std::fs::write(
                &net_json,
                content.replace("##\n# @NetdevVmnetHostOptions:", netdev_struct),
            )?;
        }

        patch_c_file(
            &net_json,
            "'vhost-vdpa': 'NetdevVhostVDPAOptions',",
            "'vhost-vdpa': 'NetdevVhostVDPAOptions',\n    'virtmcu':    'NetdevVirtmcuOptions',",
        )?;
    }

    // 2. char.json
    if char_json.exists() {
        let chardev_structs = r#"
##
# @ChardevVirtmcuOptions:
#
# virtmcu: Virtual clock chardev backend
#
# @node: The node ID
# @transport: The transport to use (zenoh or unix) (optional)
# @router: The zenoh router address (optional)
# @topic: The topic to publish/subscribe to (optional)
# @max-backlog: Maximum number of bytes in the RX backlog
#     (default: 256) (optional)
#
# Since: 11.0
##
{ 'struct': 'ChardevVirtmcuOptions',
  'base': 'ChardevCommon',
  'data': {
    'node': 'str',
    '*transport': 'str',
    '*router': 'str',
    '*topic': 'str',
    '*max-backlog': 'size' } }

##
# @ChardevVirtmcuWrapper:
#
# @data: Configuration info for virtmcu chardevs
#
# Since: 11.0
##
{ 'struct': 'ChardevVirtmcuWrapper',
  'data': { 'data': 'ChardevVirtmcuOptions' } }


##
# @ChardevFileWrapper:"#;
        let content = std::fs::read_to_string(&char_json)?;
        if !content.contains("ChardevVirtmcuOptions") {
            std::fs::write(
                &char_json,
                content.replace("##\n# @ChardevFileWrapper:", chardev_structs),
            )?;
        }

        patch_c_file(
            &char_json,
            "'ringbuf': 'ChardevRingbufWrapper',",
            "'ringbuf': 'ChardevRingbufWrapper',\n            'virtmcu': 'ChardevVirtmcuWrapper',",
        )?;
    }

    // 3. qom.json
    if qom_json.exists() {
        let can_host_virtmcu_struct = r#"
##
# @CanHostVirtmcuProperties:
#
# Properties for can-host-virtmcu objects.
#
# @node: The node ID
# @transport: The transport to use (zenoh or unix) (optional)
# @router: The zenoh router address (optional)
# @topic: The topic to publish/subscribe to
#
# @canbus: object ID of the can-bus object to connect to the host
#     interface
#
# Since: 11.0
##
{ 'struct': 'CanHostVirtmcuProperties',
  'data': { 'canbus': 'str',
            'node': 'str',
            '*transport': 'str',
            '*router': 'str',
            'topic': 'str' } }

##
# @ColoCompareProperties:"#;
        let content = std::fs::read_to_string(&qom_json)?;
        if !content.contains("CanHostVirtmcuProperties") {
            std::fs::write(
                &qom_json,
                content.replace("##\n# @ColoCompareProperties:", can_host_virtmcu_struct),
            )?;
        }

        patch_c_file(
            &qom_json,
            "    'colo-compare',",
            "    'can-host-virtmcu',\n    'colo-compare',",
        )?;
        patch_c_file(
            &qom_json,
            "      'colo-compare':               'ColoCompareProperties',",
            "      'can-host-virtmcu':             'CanHostVirtmcuProperties',\n      'colo-compare':               'ColoCompareProperties',",
        )?;
    }

    Ok(())
}

fn apply_zenoh_netdev(qemu: &Path) -> Result<()> {
    info!("  -> Injecting Zenoh netdev...");
    let net_c = qemu.join("net/net.c");
    if net_c.exists() {
        patch_c_file(
            &net_c,
            "#ifdef CONFIG_AF_XDP\n        [NET_CLIENT_DRIVER_AF_XDP]    = net_init_af_xdp,\n#endif",
            "#ifdef CONFIG_AF_XDP\n        [NET_CLIENT_DRIVER_AF_XDP]    = net_init_af_xdp,\n#endif\n        [NET_CLIENT_DRIVER_VIRTMCU]     = net_init_virtmcu,",
        )?;
    }

    let clients_h = qemu.join("net/clients.h");
    if clients_h.exists() {
        patch_c_file(
            &clients_h,
            "int net_init_socket(const Netdev *netdev, const char *name,",
            "int net_init_virtmcu(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp);\nint net_init_socket(const Netdev *netdev, const char *name,",
        )?;
    }

    let meson_build = qemu.join("net/meson.build");
    if meson_build.exists() {
        patch_c_file(
            &meson_build,
            "  'checksum.c',",
            "  'checksum.c',\n  'virtmcu.c',",
        )?;
    }

    let virtmcu_c = qemu.join("net/virtmcu.c");
    if !virtmcu_c.exists() {
        let virtmcu_c_content = r#"#include "qemu/osdep.h"
#include "net/net.h"
#include "qapi/qapi-types-net.h"
#include "clients.h"
#include "qapi/error.h"
#include "virtmcu/hooks.h"
#include "qemu/module.h"
#include "qom/object.h"

int (*virtmcu_netdev_hook)(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp) = NULL;

int net_init_virtmcu(const Netdev *netdev, const char *name, NetClientState *peer, Error **errp)
{
    /* QEMU modules are loaded by object types. Try to load the module providing netdev */
    if (!virtmcu_netdev_hook) {
        module_load_qom("netdev", NULL);
        object_class_by_name("netdev");
    }

    if (virtmcu_netdev_hook) {
        return virtmcu_netdev_hook(netdev, name, peer, errp);
    }

    error_setg(errp, "netdev module not loaded or hook not registered");
    return -1;
}
"#;
        std::fs::write(&virtmcu_c, virtmcu_c_content)?;
    }

    Ok(())
}

fn apply_zenoh_chardev(qemu: &Path) -> Result<()> {
    info!("  -> Injecting Zenoh chardev...");
    let char_c = qemu.join("chardev/char.c");
    if char_c.exists() {
        let content = std::fs::read_to_string(&char_c)?;
        if !content.contains(".name = \"max-backlog\",") {
            let re = regex::Regex::new(r#"\.name\s*=\s*"size","#).unwrap();
            if let Some(m) = re.find(&content) {
                let insertion = r#".name = "node",
            .type = QEMU_OPT_STRING,
        },{
            .name = "transport",
            .type = QEMU_OPT_STRING,
        },{
            .name = "router",
            .type = QEMU_OPT_STRING,
        },{
            .name = "topic",
            .type = QEMU_OPT_STRING,
        },{
            .name = "max-backlog",
            .type = QEMU_OPT_SIZE,
        },{
            .name = "baud-rate-ns",
            .type = QEMU_OPT_NUMBER,
        },{
            .name = "size","#;
                let new_content = content.replace(m.as_str(), insertion);
                std::fs::write(&char_c, new_content)?;
            }
        }
    }
    Ok(())
}

fn apply_fdt_generic_util_fix(qemu: &Path) -> Result<()> {
    info!("  -> Applying FDT generic util fixes...");
    let filepath = qemu.join("hw/core/fdt_generic_util.c");
    if !filepath.exists() {
        return Ok(());
    }

    let mut text = std::fs::read_to_string(&filepath)?;

    // Fix 1
    text = text.replace(
        "qemu_fdt_getprop_cell_inherited(fdti->fdt, node_path,\n                                            size_prop_name",
        "qemu_fdt_getprop_cell_inherited(fdti->fdt, pnp,\n                                            size_prop_name",
    );

    // Fix 2
    let old_bus_logic = r#"        if (object_dynamic_cast(dev, TYPE_DEVICE)) {
            Object *parent_bus = parent;
            unsigned int depth = 0;

            fdt_debug_np("bus parenting node
");
            /* Look for an FDT ancestor that is a Bus.  */
            while (parent_bus && !object_dynamic_cast(parent_bus, TYPE_BUS)) {"#;

    let new_bus_logic = r#"        if (object_dynamic_cast(dev, TYPE_DEVICE)) {
            Object *parent_bus = parent;
            DeviceClass *dc = DEVICE_GET_CLASS(dev);
            unsigned int depth = 0;

            fdt_debug_np("bus parenting node
");

            /* Task 21.7.1: Look for a child bus of the right type first */
            if (parent && object_dynamic_cast(parent, TYPE_DEVICE)) {
                DeviceState *ps = DEVICE(parent);
                BusState *b;
                QLIST_FOREACH(b, &ps->child_bus, sibling) {
                    if (!dc->bus_type || object_dynamic_cast(OBJECT(b), dc->bus_type)) {
                        parent_bus = OBJECT(b);
                        break;
                    }
                }
            }

            /* Look for an FDT ancestor that is a Bus.  */
            while (parent_bus && !object_dynamic_cast(parent_bus, TYPE_BUS)) {"#;
    text = text.replace(old_bus_logic, new_bus_logic);

    // Fix 3
    text = text.replace(
        "return be32_to_cpu(*((uint64_t *)p));",
        "return be64_to_cpu(*((uint64_t *)p));",
    );

    // Fix 4
    text = text.replace(
        "        DeviceClass *dc = DEVICE_GET_CLASS(dev);\n        const char *short_name = strrchr(node_path, '/') + 1;",
        "        const char *short_name = strrchr(node_path, '/') + 1;",
    );
    let old_realize = r#"            object_property_set_bool(OBJECT(dev), "realized", true,
                                     &error_fatal);
            if (dc->legacy_reset) {
                qemu_register_reset((void (*)(void *))dc->legacy_reset,
                                    dev);
            }
        }"#;
    let new_realize = r#"            object_property_set_bool(OBJECT(dev), "realized", true,
                                     &error_fatal);
        }"#;
    text = text.replace(old_realize, new_realize);

    // Fix 5
    text = text.replace(
        "        prop_value += elem_len;",
        "        prop_value = (const uint8_t *)prop_value + elem_len;",
    );

    // Fix 6
    let old_parenting = r#"    } else if (parent) {
        fdt_debug_np("parenting node
");
        object_property_add_child(OBJECT(parent),
                              strdup(strrchr(node_path, '/') + 1),
                              OBJECT(dev));"#;
    let new_parenting = r#"    } else if (parent) {
        char *name;
        fdt_debug_np("parenting node
");
        name = g_strdup(strrchr(node_path, '/') + 1);
        object_property_add_child(OBJECT(parent), name, OBJECT(dev));
        g_free(name);"#;
    text = text.replace(old_parenting, new_parenting);

    // Fix 7
    text = text.replace(
        "dp->node_path = strdup(node_path);",
        "dp->node_path = g_strdup(node_path);",
    );

    // Fix 8
    text = text.replace(
        "        if (device_type) {\n            if (!fdt_init_qdev(node_path, fdti, device_type)) {\n                goto exit;\n            }\n        }",
        "        if (device_type && strcmp(device_type, \"memory\") != 0 && strcmp(device_type, \"cpu\") != 0) {\n            fdt_init_qdev(node_path, fdti, device_type);\n        }",
    );

    std::fs::write(&filepath, text)?;
    Ok(())
}

fn apply_sysbus_asan_fix(qemu: &Path) -> Result<()> {
    info!("  -> Applying SysBus ASan fix...");
    let filepath = qemu.join("hw/core/sysbus.c");
    if !filepath.exists() {
        return Ok(());
    }

    let mut text = std::fs::read_to_string(&filepath)?;
    let old_logic = r#"static bool sysbus_parse_reg(FDTGenericMMap *obj, FDTGenericRegPropInfo reg,
                             Error **errp)
{
    int i;

    for (i = 0; i < reg.n; ++i) {
        MemoryRegion *mr_parent = (MemoryRegion *)
            object_dynamic_cast(reg.parents[i], TYPE_MEMORY_REGION);
        if (!mr_parent) {
            /* evil */
            mr_parent = get_system_memory();
        }
        memory_region_add_subregion_overlap(mr_parent, reg.a[i],
                                 sysbus_mmio_get_region(SYS_BUS_DEVICE(obj), i),
                                 reg.p[i]);
    }
    return false;
}"#;

    let new_logic = r#"static bool sysbus_parse_reg(FDTGenericMMap *obj, FDTGenericRegPropInfo reg,
                             Error **errp)
{
    int i;
    SysBusDevice *sbd = (SysBusDevice *)object_dynamic_cast(OBJECT(obj), TYPE_SYS_BUS_DEVICE);

    if (!sbd) {
        return false;
    }

    for (i = 0; i < reg.n; ++i) {
        MemoryRegion *mr_parent = (MemoryRegion *)
            object_dynamic_cast(reg.parents[i], TYPE_MEMORY_REGION);
        MemoryRegion *mr;

        if (!mr_parent) {
            /* evil */
            mr_parent = get_system_memory();
        }

        mr = sysbus_mmio_get_region(sbd, i);
        if (mr && !mr->container) {
            memory_region_add_subregion_overlap(mr_parent, reg.a[i],
                                     mr,
                                     reg.p[i]);
        }
    }
    return false;
}"#;

    if text.contains(old_logic) {
        text = text.replace(old_logic, new_logic);
        std::fs::write(&filepath, text)?;
    }
    Ok(())
}

fn apply_rust_asan_fix(qemu: &Path) -> Result<()> {
    info!("  -> Applying Rust ASan/UBSan fixes...");
    let meson_build = qemu.join("meson.build");
    if !meson_build.exists() {
        return Ok(());
    }

    let mut content = std::fs::read_to_string(&meson_build)?;
    let mut changed = false;

    // 1. AddressSanitizer (asan)
    let rust_asan = "add_project_arguments('-C', 'link-arg=-fsanitize=address', language: 'rust')";
    if !content.contains("get_option('b_sanitize').contains('address')") {
        let re_c = regex::Regex::new(
            r"(\s+)(qemu_cflags\s+=\s+\['-fsanitize=address'\]\s+\+\s+qemu_cflags
\s+qemu_ldflags\s+=\s+\['-fsanitize=address'\]\s+\+\s+qemu_ldflags)",
        )
        .unwrap();
        if let Some(caps) = re_c.captures(&content.clone()) {
            let indent = &caps[1];
            let original_c = &caps[2];
            let new_c = format!(
                "if not get_option('b_sanitize').contains('address')
{}  {}
{}endif",
                indent, original_c, indent
            );
            content = content.replace(original_c, &new_c);
            changed = true;
        }

        let re_rust = regex::Regex::new(&format!(
            r"(\s+)if have_rust\s*
\s*{}\s*
\s*endif",
            regex::escape(rust_asan)
        ))
        .unwrap();
        if let Some(caps) = re_rust.captures(&content.clone()) {
            let indent = &caps[1];
            let new_rust = format!(
                "{}if have_rust and not get_option('b_sanitize').contains('address')
{}  {}
{}endif",
                indent, indent, rust_asan, indent
            );
            content = content.replace(&caps[0], &new_rust);
            changed = true;
        } else if !content.contains(rust_asan) {
            let re_insert = regex::Regex::new(
                r"(if not get_option\('b_sanitize'\)\.contains\('address'\)
\s+qemu_cflags.*
\s+qemu_ldflags.*
\s+endif)",
            )
            .unwrap();
            if let Some(caps) = re_insert.captures(&content.clone()) {
                let insertion = format!(
                    "

    if have_rust and not get_option('b_sanitize').contains('address')
      {}
    endif",
                    rust_asan
                );
                content = content.replace(&caps[1], &format!("{}{}", &caps[1], insertion));
                changed = true;
            }
        }
    }

    // 2. UndefinedBehaviorSanitizer (ubsan)
    let rust_ubsan =
        "add_project_arguments('-C', 'link-arg=-fsanitize=undefined', language: 'rust')";
    if !content.contains("get_option('b_sanitize').contains('undefined')") {
        let re_c = regex::Regex::new(
            r"(\s+)(qemu_cflags\s+\+=\s+\['-fsanitize=undefined'\]
\s+qemu_ldflags\s+\+=\s+\['-fsanitize=undefined'\])",
        )
        .unwrap();
        if let Some(caps) = re_c.captures(&content.clone()) {
            let indent = &caps[1];
            let original_c = &caps[2];
            let new_c = format!(
                "if not get_option('b_sanitize').contains('undefined')
{}  {}
{}endif",
                indent, original_c, indent
            );
            content = content.replace(original_c, &new_c);
            changed = true;
        }

        let re_rust = regex::Regex::new(&format!(
            r"(\s+)if have_rust\s*
\s*{}\s*
\s*endif",
            regex::escape(rust_ubsan)
        ))
        .unwrap();
        if let Some(caps) = re_rust.captures(&content.clone()) {
            let indent = &caps[1];
            let new_rust = format!(
                "{}if have_rust and not get_option('b_sanitize').contains('undefined')
{}  {}
{}endif",
                indent, indent, rust_ubsan, indent
            );
            content = content.replace(&caps[0], &new_rust);
            changed = true;
        } else if !content.contains(rust_ubsan) {
            let re_insert = regex::Regex::new(
                r"(if not get_option\('b_sanitize'\)\.contains\('undefined'\)
\s+qemu_cflags.*
\s+qemu_ldflags.*
\s+endif)",
            )
            .unwrap();
            if let Some(caps) = re_insert.captures(&content.clone()) {
                let insertion = format!(
                    "

    if have_rust and not get_option('b_sanitize').contains('undefined')
      {}
    endif",
                    rust_ubsan
                );
                content = content.replace(&caps[1], &format!("{}{}", &caps[1], insertion));
                changed = true;
            }
        }
    }

    if changed {
        std::fs::write(&meson_build, content)?;
    }

    Ok(())
}

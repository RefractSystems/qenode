use crate::{artifacts::ArtifactCache, qmp::QmpClient, TestContext};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};
use tracing::info;

pub struct NodeConfig {
    pub id: u32,
    pub arch: String,
    pub asm: Option<String>,
    pub firmware_path: Option<String>,
    pub dts: Option<String>,
    pub dts_content: Option<String>,
    pub dts_path: Option<String>,
    pub dtb_path: Option<String>,
    pub yaml_path: Option<String>,
    pub qemu_args: Vec<String>,
    pub is_coordinated: bool,
}

impl NodeConfig {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            arch: "arm".to_string(),
            asm: None,
            firmware_path: None,
            dts: None,
            dts_content: None,
            dts_path: None,
            dtb_path: None,
            yaml_path: None,
            qemu_args: vec![
                "-nographic".to_string(),
                "-monitor".to_string(),
                "none".to_string(),
            ],
            is_coordinated: true,
        }
    }

    pub fn with_arch(mut self, arch: &str) -> Self {
        self.arch = arch.to_string();
        self
    }

    pub fn orchestrated(mut self, coordinated: bool) -> Self {
        self.is_coordinated = coordinated;
        self
    }

    pub fn with_firmware_asm(mut self, asm: &str) -> Self {
        self.asm = Some(asm.to_string());
        self
    }

    pub fn with_firmware_path(mut self, path: &str) -> Self {
        self.firmware_path = Some(path.to_string());
        self
    }

    pub fn with_dtb(mut self, dts: &str) -> Self {
        self.dts = Some(dts.to_string());
        self
    }

    pub fn with_dtb_path(mut self, path: &str) -> Self {
        self.dtb_path = Some(path.to_string());
        self
    }

    pub fn with_dts_content(mut self, content: &str) -> Self {
        self.dts_content = Some(content.to_string());
        self
    }

    pub fn with_yaml_path(mut self, path: &str) -> Self {
        self.yaml_path = Some(path.to_string());
        self
    }

    pub fn with_dts_path(mut self, path: &str) -> Self {
        self.dts_path = Some(path.to_string());
        self
    }

    pub fn add_qemu_arg(mut self, arg: &str) -> Self {
        self.qemu_args.push(arg.to_string());
        self
    }
}

pub struct TopologyBuilder {
    nodes: Vec<NodeConfig>,
    timeout_secs: u64,
    variables: std::collections::HashMap<String, String>,
    federation_id: Option<String>,
    transport_override: Option<String>,
}

impl Default for TopologyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TopologyBuilder {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            timeout_secs: 10,
            variables: std::collections::HashMap::new(),
            federation_id: None,
            transport_override: None,
        }
    }

    pub fn with_transport_override(mut self, transport: &str) -> Self {
        self.transport_override = Some(transport.to_string());
        self
    }

    pub fn with_federation_id(mut self, id: &str) -> Self {
        self.federation_id = Some(id.to_string());
        self
    }

    /// SOTA Async Teardown: Builds the environment and executes a test closure,
    /// guaranteeing graceful teardown even on panic.
    pub async fn run_test<F>(self, test_func: F) -> Result<()>
    where
        F: for<'a> FnOnce(&'a mut VirtmcuTestEnv) -> futures::future::BoxFuture<'a, Result<()>>,
    {
        let env = self.build().await?;
        env.run_test(test_func).await
    }

    pub fn with_variable(mut self, key: &str, value: &str) -> Self {
        self.variables.insert(key.to_string(), value.to_string());
        self
    }

    pub fn add_node(mut self, node: NodeConfig) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    async fn spawn_zenoh_coordinator(
        ctx: &TestContext,
        endpoint: &str,
    ) -> Result<tokio::process::Child> {
        let router_bin = ctx.find_binary("zenoh_coordinator")?;
        info!("Spawning zenoh_coordinator from: {}", router_bin.display());

        let mut router_cmd = Command::new(&router_bin);
        router_cmd.arg("--listen").arg(endpoint).kill_on_drop(true);

        let router_proc = router_cmd.spawn().map_err(|e| {
            let mut extra_hint = String::new();
            if e.kind() == std::io::ErrorKind::NotFound && router_bin.exists() {
                extra_hint = format!(
                    "\nNote: Binary exists at {} but spawn failed with 'Not Found'. \
                     This often means the binary was built for a different architecture (e.g. x86_64 vs aarch64) \
                     or its dynamic linker is missing. Try running 'cargo build -p zenoh_coordinator' in this environment.",
                    router_bin.display()
                );
            }
            anyhow!(
                "Failed to spawn native zenoh_coordinator at {}: {}. {}. \n\
                Hint: Ensure zenoh_coordinator is built for the current architecture and its dependencies are available.",
                router_bin.display(),
                e,
                extra_hint
            )
        })?;
        tokio::time::sleep(Duration::from_millis(2000)).await;
        Ok(router_proc)
    }

    pub async fn build(self) -> Result<VirtmcuTestEnv> {
        if self.nodes.is_empty() {
            return Err(anyhow!("Topology must have at least one node"));
        }

        let mut ctx = TestContext::new()?;
        for (k, v) in self.variables {
            ctx.variables.insert(k, v);
        }

        let transport = self.transport_override.as_deref().unwrap_or("zenoh");
        let is_unix = transport == "unix";

        let (router_child, endpoint) = if is_unix {
            let sim_id = self.federation_id.as_deref().unwrap_or("default-fed");
            let sock_path = ctx.tmp_path(&format!("{}/coordinator.sock", sim_id));
            let endpoint = sock_path.to_string_lossy().to_string();

            // Find a valid YAML path from the nodes to pass to the coordinator
            let mut topo_path_str = String::new();
            for node in &self.nodes {
                if let Some(path) = &node.yaml_path {
                    let full_path = ctx.workspace_root.join(path);
                    topo_path_str = full_path.to_string_lossy().to_string();
                    break;
                }
            }
            if topo_path_str.is_empty() {
                // Fallback to basic generated topology if no YAML is provided
                let mut topo_nodes = String::new();
                for node in &self.nodes {
                    topo_nodes.push_str(&format!("    - name: '{}'\n", node.id));
                }
                let topo_yaml = format!(
                    "topology:
  transport: unix
  nodes:
{}
",
                    topo_nodes
                );
                let topo_path = ctx.tmp_path("coordinator_topo.yaml");
                std::fs::write(&topo_path, topo_yaml).context("Failed to write topo_yaml")?;
                topo_path_str = topo_path.to_string_lossy().to_string();
            }

            // Spawn deterministic_coordinator for UDS
            let coordinator_bin = ctx.find_binary("deterministic_coordinator")?;
            info!(
                "Spawning deterministic_coordinator (UDS) from: {}",
                coordinator_bin.display()
            );

            let mut coord_cmd = Command::new(&coordinator_bin);
            coord_cmd
                .arg("--transport")
                .arg("unix")
                .arg("--nodes")
                .arg(self.nodes.len().to_string())
                .arg("--federation-id")
                .arg(self.federation_id.as_deref().unwrap_or("default-fed"))
                .arg("--run-dir")
                .arg(ctx.tmp_dir.path().to_string_lossy().to_string())
                .arg("--topology")
                .arg(&topo_path_str)
                .arg("--join-timeout-ms")
                .arg("10000")
                .kill_on_drop(true);

            let coord_proc = coord_cmd
                .spawn()
                .context("Failed to spawn deterministic_coordinator")?;
            // Give it time to bind the socket
            tokio::time::sleep(Duration::from_millis(1000)).await;
            (Some(coord_proc), endpoint)
        } else {
            let endpoint = ctx.variables.get("ROUTER_ENDPOINT").unwrap().clone();
            (
                Some(Self::spawn_zenoh_coordinator(&ctx, &endpoint).await?),
                endpoint,
            )
        };
        // Update ctx variables with the final endpoint
        ctx.variables
            .insert("ROUTER_ENDPOINT".to_string(), endpoint.clone());

        let artifacts = ArtifactCache::new(ctx.workspace_root.clone())?;
        let launcher = crate::launcher::QemuLauncher::new(ctx.workspace_root.clone());

        let mut qemu_procs = Vec::new();
        let mut uart_readers = Vec::new();
        let mut qmp_clients = Vec::new();
        let mut pgids = Vec::new();
        let mut is_coordinated_flags = Vec::new();
        let mut recent_qemu_stderr = Vec::new();

        let qmp_socket_paths: Vec<PathBuf> = self
            .nodes
            .iter()
            .map(|n| ctx.tmp_path(&format!("qmp_{}.sock", n.id)))
            .collect();
        let coordinated_nodes: Vec<u32> = self
            .nodes
            .iter()
            .filter(|n| n.is_coordinated)
            .map(|n| n.id)
            .collect();

        for node in &self.nodes {
            is_coordinated_flags.push(node.is_coordinated);
            let qemu_bin = launcher.resolve_qemu_bin(&node.arch)?;

            let elf_path = if let Some(asm) = &node.asm {
                artifacts.get_firmware_asm(asm).await?
            } else if let Some(p) = &node.firmware_path {
                ctx.workspace_root.join(p)
            } else {
                return Err(anyhow!("Firmware ASM or path is required"));
            };

            let mut yaml_cli_args = Vec::new();

            let dtb_path = if let Some(dts) = &node.dts {
                let sub_dts = ctx.substitute(dts);
                artifacts.get_dtb_dts(&sub_dts).await?
            } else if let Some(content) = &node.dts_content {
                let sub_dts = ctx.substitute(content);
                artifacts.get_dtb_dts(&sub_dts).await?
            } else if let Some(p) = &node.dts_path {
                let dts_content = std::fs::read_to_string(ctx.workspace_root.join(p))
                    .context(format!("Failed to read DTS path: {}", p))?;
                let sub_dts = ctx.substitute(&dts_content);
                artifacts.get_dtb_dts(&sub_dts).await?
            } else if let Some(p) = &node.dtb_path {
                ctx.workspace_root.join(p)
            } else if let Some(p) = &node.yaml_path {
                let mut yaml_content = std::fs::read_to_string(ctx.workspace_root.join(p))
                    .context(format!("Failed to read YAML path: {}", p))?;

                if let Some(t) = &self.transport_override {
                    if yaml_content.contains("transport:") {
                        yaml_content =
                            yaml_content.replace("transport: zenoh", &format!("transport: {}", t));
                        yaml_content =
                            yaml_content.replace("transport: unix", &format!("transport: {}", t));
                    } else {
                        yaml_content = yaml_content
                            .replace("topology:\n", &format!("topology:\n  transport: {}\n", t));
                        // Also try CRLF just in case
                        yaml_content = yaml_content.replace(
                            "topology:\r\n",
                            &format!("topology:\r\n  transport: {}\r\n", t),
                        );
                    }
                }

                let yaml_content = yaml_content.replace("ZENOH_ROUTER_ENDPOINT", &endpoint);
                let yaml_content = ctx.substitute(&yaml_content);

                std::env::set_var("VIRTMCU_WORKSPACE", &ctx.workspace_root);
                std::env::set_var("VIRTMCU_TRANSPORT", transport);
                let (platform, world) =
                    yaml2qemu::parse_yaml(&yaml_content, Some(&endpoint), node.id)?;

                yaml_cli_args.clear();
                yaml_cli_args = platform.cli_args;
                let dtb = artifacts.get_dtb_dts(&platform.dts_content).await?;
                yaml2qemu::validate_dtb(&dtb, &world)?;
                dtb
            } else {
                return Err(anyhow!(
                    "Device Tree DTS, DTB path, or YAML path is required"
                ));
            };

            let mut qemu_cmd = Command::new(&qemu_bin);

            for arg in &yaml_cli_args {
                qemu_cmd.arg(arg);
            }

            let mut machine = "arm-generic-fdt".to_string();
            let mut dtb_arg = format!("hw-dtb={}", dtb_path.display());

            if node.arch.starts_with("riscv") {
                machine = "virt".to_string();
                dtb_arg = "".to_string();
            }

            qemu_cmd.env("LD_LIBRARY_PATH", launcher.get_ld_library_path());
            if let Some(mdir) = launcher.get_module_dir() {
                qemu_cmd.env("QEMU_MODULE_DIR", mdir);
            }

            if std::env::var("GCOV_PREFIX").is_err() {
                qemu_cmd.env(
                    "GCOV_PREFIX",
                    format!(
                        "{}/target/coverage/{}_{}_node{}",
                        ctx.workspace_root.display(),
                        node.arch,
                        launcher.build_dir_name(),
                        node.id
                    ),
                );
            }

            if !is_unix {
                qemu_cmd.env("ZENOH_ROUTER_ENDPOINT", &endpoint);
                qemu_cmd.env("VIRTMCU_ZENOH_ROUTER", &endpoint);
            } else {
                qemu_cmd.env("VIRTMCU_COORD_SOCK", &endpoint);
                qemu_cmd.env(
                    "VIRTMCU_SIM_ID",
                    self.federation_id.as_deref().unwrap_or("default-fed"),
                );
            }

            let qmp_sock_path = ctx.tmp_path(&format!("qmp_{}.sock", node.id));
            qemu_cmd.arg("-qmp").arg(format!(
                "unix:{},server=on,wait=off",
                qmp_sock_path.display()
            ));

            if dtb_arg.is_empty() {
                qemu_cmd.arg("-M").arg(&machine);
                qemu_cmd.arg("-dtb").arg(&dtb_path);
            } else {
                qemu_cmd.arg("-M").arg(format!("{},{}", machine, dtb_arg));
            }

            qemu_cmd.arg("-kernel").arg(&elf_path);

            let has_manual_clock = node
                .qemu_args
                .iter()
                .any(|arg| arg.contains("virtmcu-clock"));
            if node.is_coordinated {
                // Link all native devices to the hub (hub0 is created by yaml2qemu in DTS)
                for dev_type in &[
                    "actuator",
                    "sensor",
                    "telemetry",
                    "ieee802154",
                    "canfd",
                    "flexray",
                    "lin",
                    "spi",
                    "wifi",
                    "ui",
                    "reference-peripheral",
                ] {
                    qemu_cmd
                        .arg("-global")
                        .arg(format!("{}.transport=hub0", dev_type));
                }
                let has_clock_in_yaml_args = yaml_cli_args
                    .iter()
                    .any(|arg| arg.contains("virtmcu-clock"));
                if !has_manual_clock && !has_clock_in_yaml_args {
                    let mode = if is_unix {
                        "slaved-unix"
                    } else {
                        "slaved-suspend"
                    };
                    let clock_router = if is_unix {
                        ctx.tmp_path(&format!("clock_{}.sock", node.id))
                            .to_string_lossy()
                            .to_string()
                    } else {
                        endpoint.clone()
                    };
                    let mut clock_arg = format!(
                        "virtmcu-clock,mode={},router={},node={}",
                        mode, clock_router, node.id
                    );
                    if is_unix {
                        clock_arg.push_str(&format!(
                            ",coordinated=on,coordinated-router={},federation-id={}",
                            endpoint,
                            self.federation_id.as_deref().unwrap_or("default-fed")
                        ));
                    }
                    qemu_cmd.arg("-device").arg(clock_arg);
                }
                qemu_cmd.arg("-S");
            }

            let uart_sock_path = ctx.tmp_path(&format!("uart_{}.sock", node.id));
            qemu_cmd.arg("-serial").arg(format!(
                "unix:{},server=on,wait=on",
                uart_sock_path.display()
            ));

            for arg in &node.qemu_args {
                qemu_cmd.arg(ctx.substitute(arg));
            }

            unsafe {
                qemu_cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }

            ctx.setup_cmd(&mut qemu_cmd);
            qemu_cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            println!("Spawning QEMU with command: {:?}", qemu_cmd);
            info!("Spawning QEMU with command: {:?}", qemu_cmd);
            let mut qemu = qemu_cmd
                .spawn()
                .context("Failed to spawn virtmcu-run (QEMU)")?;

            let pgid = qemu.id().map(|pid| unsafe { libc::getpgid(pid as i32) });
            pgids.push(pgid);

            let stderr = qemu.stderr.take().unwrap();
            let mut stderr_lines = BufReader::new(stderr).lines();
            let recent_stderr = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
            recent_qemu_stderr.push(recent_stderr.clone());

            let node_id_for_log = node.id;
            let recent_stderr_for_spawn = recent_stderr.clone();
            tokio::spawn(async move {
                while let Ok(Some(line)) = stderr_lines.next_line().await {
                    eprintln!("[QEMU {}] {}", node_id_for_log, line);
                    let mut recent = recent_stderr_for_spawn.lock().await;
                    recent.push(line.clone());
                    if recent.len() > 1000 {
                        recent.remove(0);
                    }

                    // Detect unknown QOM property names early. QEMU silently ignores
                    // unknown properties — the device gets a null/default value and the
                    // failure surfaces 30+ seconds later as CLOCK_ERROR_STALL or a
                    // coordinator federation_id mismatch. Common cause: underscore instead
                    // of hyphen in -device arguments (e.g. federation_id= vs federation-id=).
                    if line.contains("not found")
                        && (line.contains("Property '") || line.contains("property '"))
                    {
                        tracing::error!(
                            "[QEMU] [Node {}] UNKNOWN QOM PROPERTY DETECTED — check that \
                             -device argument names use hyphens not underscores. \
                             This will cause silent null/default values and a downstream stall: {}",
                            node_id_for_log,
                            line
                        );
                    } else if line.contains("[ERROR]")
                        || line.contains("error:")
                        || line.contains("fatal:")
                        || line.contains("panic")
                    {
                        tracing::error!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[WARN ]") || line.contains("warning:") {
                        tracing::warn!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[DEBUG]") {
                        tracing::debug!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[TRACE]") {
                        tracing::trace!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[INFO ]") || line.contains("info:") {
                        tracing::info!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else {
                        tracing::info!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    }
                }
            });

            // Connect to UART
            let mut found = false;
            for _ in 0..50 {
                if uart_sock_path.exists() {
                    found = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if !found {
                return Err(anyhow!("Timeout waiting for UART socket"));
            }

            let uart_stream_res = tokio::net::UnixStream::connect(&uart_sock_path).await;
            let uart_stream = match uart_stream_res {
                Ok(s) => s,
                Err(e) => {
                    let stderr_lock = recent_stderr.lock().await;
                    let last_lines = stderr_lock.join("\n");
                    return Err(anyhow!(
                        "Failed to connect to UART socket at {}: {}. [QEMU Stderr]:\n{}",
                        uart_sock_path.display(),
                        e,
                        last_lines
                    ));
                }
            };
            uart_readers.push(BufReader::new(uart_stream));

            // Connect to QMP
            let mut found = false;
            for _ in 0..50 {
                if qmp_sock_path.exists() {
                    found = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if !found {
                let stderr_lock = recent_stderr.lock().await;
                let last_lines = stderr_lock.join("\n");
                return Err(anyhow!(
                    "Timeout waiting for QMP socket at {}. [QEMU Stderr]:\n{}",
                    qmp_sock_path.display(),
                    last_lines
                ));
            }

            let qmp_res = QmpClient::connect(&qmp_sock_path).await;
            let qmp = match qmp_res {
                Ok(q) => q,
                Err(e) => {
                    let stderr_lock = recent_stderr.lock().await;
                    let last_lines = stderr_lock.join("\n");
                    return Err(anyhow!(
                        "Failed to connect to QMP socket at {}: {}. [QEMU Stderr]:\n{}",
                        qmp_sock_path.display(),
                        e,
                        last_lines
                    ));
                }
            };
            qmp_clients.push(qmp);

            qemu_procs.push(qemu);
        }

        let mut session_opt = None;
        let clock_coordinator: std::sync::Arc<dyn ClockCoordinator>;

        if !is_unix {
            let mut zconfig = zenoh::Config::default();
            zconfig
                .insert_json5("connect/endpoints", &format!("[\"{}\"]", endpoint))
                .map_err(|e| anyhow!("Config error: {}", e))?;
            zconfig
                .insert_json5("scouting/multicast/enabled", "false")
                .map_err(|e| anyhow!("Config error: {}", e))?;
            zconfig
                .insert_json5("mode", "\"client\"")
                .map_err(|e| anyhow!("Config error: {}", e))?;
            let session = zenoh::open(zconfig)
                .await
                .map_err(|e| anyhow!("Zenoh error: {}", e))?;

            // 2. Wait for Clock Liveliness for all coordinated nodes
            for node_id in &coordinated_nodes {
                let hb_topic = format!("sim/clock/liveliness/{}", node_id);
                info!("Waiting for Zenoh Liveliness heartbeat on {}...", hb_topic);

                let success = timeout(Duration::from_secs(15), async {
                    loop {
                        let mut found = false;
                        let replies = session
                            .liveliness()
                            .get(&hb_topic)
                            .await
                            .map_err(|e| anyhow!("Zenoh query failed: {}", e))?;
                        while let Ok(reply) = replies.recv_async().await {
                            if reply.result().is_ok() {
                                found = true;
                                break;
                            }
                        }
                        if found {
                            return Ok::<(), anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await;

                if success.is_err() {
                    for p in pgids.iter().flatten() {
                        unsafe {
                            libc::kill(-*p, libc::SIGKILL);
                        }
                    }
                    return Err(anyhow!(
                        "Timed out waiting for QEMU clock liveliness barrier on node {}",
                        node_id
                    ));
                }
            }

            if !coordinated_nodes.is_empty() {
                info!("Liveliness barrier passed. Executing 0-ns VTA sync for Zenoh...");
            }
            session_opt = Some(session.clone());
            clock_coordinator = std::sync::Arc::new(ZenohClockCoordinator::new(session));
        } else {
            let mut coordinator = UnixClockCoordinator::new();
            for node_id in &coordinated_nodes {
                let sock_path = ctx.tmp_path(&format!("clock_{}.sock", node_id));
                coordinator.add_node(*node_id as usize, &sock_path.to_string_lossy());
            }
            clock_coordinator = std::sync::Arc::new(coordinator);
        }

        if !coordinated_nodes.is_empty() {
            info!("Executing 0-ns VTA sync...");

            let mut sync_futures = Vec::new();
            for node_id in &coordinated_nodes {
                let cc = clock_coordinator.clone();
                let nid = *node_id as usize;
                sync_futures.push(tokio::spawn(
                    async move { cc.step_clock(nid, 0, 0, 0).await },
                ));
            }

            for handle in sync_futures {
                handle.await??;
            }

            info!("VTA Sync passed. Unfreezing all coordinated QEMUs via QMP...");
        }

        for (idx, qmp) in qmp_clients.iter_mut().enumerate() {
            // Only issue cont if it was spawned with -S (coordinated)
            if self.nodes[idx].is_coordinated {
                qmp.cont().await?;
            }
        }

        info!("All nodes unfrozen.");

        let num_nodes = self.nodes.len();
        Ok(VirtmcuTestEnv {
            ctx,
            qemu_children: qemu_procs,
            qemu_pgids: pgids,
            uart_readers,
            uart_buffers: vec![String::new(); num_nodes],
            qmp_clients,
            qmp_socket_paths,
            timeout_secs: self.timeout_secs,
            _session: session_opt,
            clock_coordinator,
            router_child,
            external_children: Vec::new(),
            is_coordinated: is_coordinated_flags,
            current_vtime: 0,
            current_quantum: 0,
            recent_qemu_stderr,
        })
    }
}

use async_trait::async_trait;

#[async_trait]
pub trait ClockCoordinator: Send + Sync {
    async fn step_clock(
        &self,
        node_id: usize,
        step_ns: u64,
        current_vtime: u64,
        quantum: u64,
    ) -> Result<()>;
}

pub struct DummyClockCoordinator;

#[async_trait]
impl ClockCoordinator for DummyClockCoordinator {
    async fn step_clock(
        &self,
        _node_id: usize,
        _step_ns: u64,
        _current_vtime: u64,
        _quantum: u64,
    ) -> Result<()> {
        Ok(())
    }
}

pub struct ZenohClockCoordinator {
    session: zenoh::Session,
}

impl ZenohClockCoordinator {
    pub fn new(session: zenoh::Session) -> Self {
        Self { session }
    }
}

#[async_trait]
impl ClockCoordinator for ZenohClockCoordinator {
    async fn step_clock(
        &self,
        node_id: usize,
        step_ns: u64,
        current_vtime: u64,
        quantum: u64,
    ) -> Result<()> {
        let advance_topic = format!("sim/clock/advance/{}", node_id);
        let mut payload = Vec::with_capacity(24);
        payload.extend_from_slice(&step_ns.to_le_bytes());
        payload.extend_from_slice(&current_vtime.to_le_bytes()); // target
        payload.extend_from_slice(&quantum.to_le_bytes()); // quantum ignored by clock plugin or incremented

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let replies = self
                .session
                .get(&advance_topic)
                .payload(payload)
                .await
                .map_err(|e| anyhow!("Zenoh query failed: {}", e))?;

            let mut got_reply = false;
            while let Ok(reply) = replies.recv_async().await {
                if reply.result().is_ok() {
                    got_reply = true;
                    break;
                }
            }

            if !got_reply {
                return Err(anyhow!(
                    "Failed to receive clock step reply for node {}",
                    node_id
                ));
            }
            Ok(())
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("Zenoh clock step timeout for node {}", node_id)))
    }
}

pub struct UnixClockCoordinator {
    transports: std::collections::HashMap<
        usize,
        std::sync::Arc<virtmcu_wire::UnixSocketPhysicalNodeTransport>,
    >,
}

impl Default for UnixClockCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl UnixClockCoordinator {
    pub fn new() -> Self {
        Self {
            transports: std::collections::HashMap::new(),
        }
    }

    pub fn add_node(&mut self, node_id: usize, path: &str) {
        let transport = virtmcu_wire::UnixSocketPhysicalNodeTransport::new(path)
            .expect("failed to bind UDS clock socket");
        self.transports
            .insert(node_id, std::sync::Arc::new(transport));
    }
}

#[async_trait]
impl ClockCoordinator for UnixClockCoordinator {
    async fn step_clock(
        &self,
        node_id: usize,
        step_ns: u64,
        current_vtime: u64,
        quantum: u64,
    ) -> Result<()> {
        let transport = self
            .transports
            .get(&node_id)
            .ok_or_else(|| anyhow!("No clock transport for node {}", node_id))?
            .clone();

        let req = virtmcu_wire::ClockAdvanceReq::new(step_ns, current_vtime, quantum);

        // Use spawn_blocking for the synchronous advance call
        tokio::task::spawn_blocking(move || {
            use virtmcu_wire::PhysicalNodeTransport;
            match transport.advance(req, Duration::from_secs(10)) {
                Some(resp) if resp.error_code() == 0 => Ok(()),
                Some(resp) => Err(anyhow!("Clock error code: {}", resp.error_code())),
                None => Err(anyhow!("Clock timeout on node {}", node_id)),
            }
        })
        .await?
    }
}

pub struct VirtmcuTestEnv {
    #[allow(dead_code)]
    ctx: TestContext,
    pub qemu_children: Vec<Child>,
    pub qemu_pgids: Vec<Option<i32>>,
    uart_readers: Vec<BufReader<tokio::net::UnixStream>>,
    pub uart_buffers: Vec<String>,
    pub qmp_clients: Vec<QmpClient>,
    #[allow(dead_code)]
    qmp_socket_paths: Vec<PathBuf>,
    timeout_secs: u64,
    #[allow(dead_code)]
    _session: Option<zenoh::Session>,
    clock_coordinator: std::sync::Arc<dyn ClockCoordinator>,
    router_child: Option<Child>,
    pub external_children: Vec<Child>,
    is_coordinated: Vec<bool>,
    pub current_vtime: u64,
    pub current_quantum: u64,
    recent_qemu_stderr: Vec<std::sync::Arc<tokio::sync::Mutex<Vec<String>>>>,
}

impl VirtmcuTestEnv {
    pub fn builder() -> TopologyBuilder {
        TopologyBuilder::new()
    }

    pub fn node_uart(&mut self, node_id: usize) -> &mut BufReader<tokio::net::UnixStream> {
        &mut self.uart_readers[node_id]
    }

    pub fn qmp(&mut self, node_id: usize) -> &mut QmpClient {
        &mut self.qmp_clients[node_id]
    }

    pub fn vtime(&self) -> u64 {
        self.current_vtime
    }

    /// Registers an external child process to be killed during environment teardown.
    pub fn register_child(&mut self, child: Child) {
        self.external_children.push(child);
    }

    /// Declares a subscriber and waits briefly to ensure Zenoh discovery is complete.
    pub async fn safe_subscribe(
        &self,
        topic: &str,
    ) -> Result<zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>>
    {
        if let Some(session) = &self._session {
            let sub = session
                .declare_subscriber(topic)
                .await
                .map_err(|e| anyhow!("Failed to declare subscriber: {}", e))?;

            // Wait for subscriber discovery to propagate.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            Ok(sub)
        } else {
            Err(anyhow!(
                "safe_subscribe called but no Zenoh session is active (UDS mode)"
            ))
        }
    }

    pub fn session(&self) -> zenoh::Session {
        self._session
            .clone()
            .expect("session() called but no Zenoh session is active (UDS mode)")
    }

    pub fn tmp_path(&self, name: &str) -> PathBuf {
        self.ctx.tmp_path(name)
    }

    pub fn find_binary(&self, name: &str) -> Result<PathBuf> {
        self.ctx.find_binary(name)
    }

    /// Returns the Zenoh router endpoint used by this environment.
    pub fn router_endpoint(&self) -> Option<String> {
        self.ctx.variables.get("ROUTER_ENDPOINT").cloned()
    }

    /// Returns the current contents of the UART buffer for a node.
    pub async fn uart_buffer(&self, node_id: usize) -> String {
        self.uart_buffers[node_id].clone()
    }

    async fn format_qemu_error(&self, node_id: usize, msg: &str) -> String {
        let stderr_lock = self.recent_qemu_stderr[node_id].lock().await;
        let last_lines = stderr_lock.join("\n");
        format!("{} [Node {} Stderr]:\n{}", msg, node_id, last_lines)
    }

    /// Advances the virtual clock for all coordinated nodes by the specified duration.
    pub async fn step_clock(&mut self, total_ns: u64, step_ns: u64) -> Result<()> {
        let mut advanced: u64 = 0;

        while advanced < total_ns {
            // Check if any QEMU crashed
            for child in &mut self.qemu_children {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(anyhow!(
                        "QEMU process died unexpectedly with status: {}",
                        status
                    ));
                }
            }

            let advance = std::cmp::min(step_ns, total_ns - advanced);
            advanced += advance;
            self.current_vtime += advance;
            let current_quantum_val = self.current_quantum;

            let mut step_futures = Vec::new();
            for node_id in 0..self.qemu_children.len() {
                if !self.is_coordinated[node_id] {
                    continue;
                }

                let cc = self.clock_coordinator.clone();
                let current_vtime_val = self.current_vtime;
                let advance_val = advance;
                step_futures.push(async move {
                    cc.step_clock(node_id, advance_val, current_vtime_val, current_quantum_val)
                        .await
                });
            }
            futures::future::try_join_all(step_futures).await?;
            self.current_quantum += 1;

            // Small yield to allow async tasks (like monitors) to process
            tokio::task::yield_now().await;
        }
        Ok(())
    }
    /// Waits for a specific string to appear in QEMU's UART without stepping the virtual clock.
    pub async fn wait_for_output_passive(&mut self, node_id: usize, pattern: &str) -> Result<()> {
        let timeout_duration = Duration::from_secs(self.timeout_secs);
        let start_time = tokio::time::Instant::now();

        // First check if the pattern is already in the buffer from previous reads
        if self.uart_buffers[node_id].contains(pattern) {
            return Ok(());
        }
        loop {
            if start_time.elapsed() > timeout_duration {
                return Err(anyhow!(
                    "Timed out waiting for {}. Buffer: {}",
                    pattern,
                    self.uart_buffers[node_id]
                ));
            }

            // Check if any QEMU crashed
            for (idx, child) in self.qemu_children.iter_mut().enumerate() {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(anyhow!(
                        self.format_qemu_error(
                            idx,
                            &format!("QEMU process died unexpectedly with status: {}", status)
                        )
                        .await
                    ));
                }
            }

            // Try to read from UART
            let uart_reader = &mut self.uart_readers[node_id];
            let uart_read = timeout(Duration::from_millis(50), async {
                use tokio::io::AsyncReadExt;
                let mut buf = [0u8; 1024];
                uart_reader.read(&mut buf).await.map(|n| (n, buf))
            })
            .await;

            if let Ok(res) = uart_read {
                match res {
                    Ok((bytes_read, buf)) => {
                        if bytes_read == 0 {
                            return Err(anyhow!(
                                self.format_qemu_error(node_id, "UART socket reached EOF")
                                    .await
                            ));
                        }
                        let chunk = String::from_utf8_lossy(&buf[..bytes_read]);
                        self.uart_buffers[node_id].push_str(&chunk);

                        let trimmed = chunk.trim();
                        if !trimmed.is_empty() {
                            info!("Node {} UART (passive): {}", node_id, trimmed);
                        }

                        if self.uart_buffers[node_id].contains(pattern) {
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!(
                            self.format_qemu_error(node_id, &format!("UART read failed: {}", e))
                                .await
                        ));
                    }
                }
            }
        }
    }

    /// Waits for a specific string to appear in QEMU's UART by actively stepping the virtual clock.
    pub async fn wait_for_output(&mut self, node_id: usize, pattern: &str) -> Result<()> {
        let timeout_duration = Duration::from_secs(self.timeout_secs);
        let start_time = tokio::time::Instant::now();

        // First check if the pattern is already in the buffer from previous reads
        if self.uart_buffers[node_id].contains(pattern) {
            return Ok(());
        }
        let step_ns: u64 = 50_000_000; // 50ms step

        loop {
            if start_time.elapsed() > timeout_duration {
                return Err(anyhow!(
                    "Timed out waiting for {}. Buffer: {}",
                    pattern,
                    self.uart_buffers[node_id]
                ));
            }

            // Check if any QEMU crashed
            for (idx, child) in self.qemu_children.iter_mut().enumerate() {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(anyhow!(
                        self.format_qemu_error(
                            idx,
                            &format!("QEMU process died unexpectedly with status: {}", status)
                        )
                        .await
                    ));
                }
            }

            // 1. Advance the clock by 1 quantum (if coordinated)
            let any_coordinated = self.is_coordinated.iter().any(|&c| c);
            if any_coordinated {
                self.current_vtime += step_ns;
                let current_quantum_val = self.current_quantum;

                let mut step_futures = Vec::new();
                for node_idx in 0..self.qemu_children.len() {
                    if self.is_coordinated[node_idx] {
                        let cc = self.clock_coordinator.clone();
                        let current_vtime_val = self.current_vtime;
                        step_futures.push(async move {
                            cc.step_clock(node_idx, step_ns, current_vtime_val, current_quantum_val)
                                .await
                        });
                    }
                }
                futures::future::try_join_all(step_futures).await?;
                self.current_quantum += 1;
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }

            // 2. Try to read from UART
            let uart_reader = &mut self.uart_readers[node_id];
            let uart_read = timeout(Duration::from_millis(5), async {
                use tokio::io::AsyncReadExt;
                let mut buf = [0u8; 1024];
                uart_reader.read(&mut buf).await.map(|n| (n, buf))
            })
            .await;

            if let Ok(res) = uart_read {
                match res {
                    Ok((bytes_read, buf)) => {
                        if bytes_read == 0 {
                            return Err(anyhow!(
                                self.format_qemu_error(node_id, "UART socket reached EOF")
                                    .await
                            ));
                        }
                        if bytes_read > 0 {
                            let chunk = String::from_utf8_lossy(&buf[..bytes_read]);
                            self.uart_buffers[node_id].push_str(&chunk);

                            let trimmed = chunk.trim();
                            if !trimmed.is_empty() {
                                info!("Node {} UART: {}", node_id, trimmed);
                            }

                            if self.uart_buffers[node_id].contains(pattern) {
                                return Ok(());
                            }
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!(
                            self.format_qemu_error(node_id, &format!("UART read failed: {}", e))
                                .await
                        ));
                    }
                }
            }
        }
    }
    pub async fn teardown(mut self) {
        // Attempt Graceful QMP shutdown (RAII SOTA) asynchronously
        for qmp in &mut self.qmp_clients {
            let _ = qmp.quit().await;
        }

        // Give QEMU instances a brief moment to exit cleanly
        tokio::time::sleep(Duration::from_millis(100)).await;

        for child in &mut self.qemu_children {
            let _ = child.kill().await;
        }
        if let Some(child) = &mut self.router_child {
            let _ = child.kill().await;
        }

        // Clear children to avoid double-kill in Drop
        self.qemu_children.clear();
    }

    /// SOTA Async Teardown: Executes a test closure and guarantees graceful teardown even on panic.
    pub async fn run_test<F>(mut self, test_func: F) -> Result<()>
    where
        F: for<'a> FnOnce(&'a mut Self) -> futures::future::BoxFuture<'a, Result<()>>,
    {
        use futures::FutureExt;

        let res = std::panic::AssertUnwindSafe(test_func(&mut self))
            .catch_unwind()
            .await;

        if !matches!(res, Ok(Ok(_))) {
            for node_id in 0..self.qemu_children.len() {
                let stderr_log = self.format_qemu_error(node_id, "Test failed").await;
                eprintln!("{}", stderr_log);
            }
        }

        self.teardown().await;

        match res {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}

impl Drop for VirtmcuTestEnv {
    fn drop(&mut self) {
        // Fallback kill for processes to guarantee panic safety.
        // We do not use blocking network calls here to prevent reactor stalls.
        for pgid in self.qemu_pgids.iter().flatten() {
            if *pgid > 1 {
                unsafe {
                    libc::kill(-*pgid, libc::SIGKILL);
                }
            }
        }

        for child in &mut self.qemu_children {
            let _ = child.start_kill();
        }
        for child in &mut self.external_children {
            let _ = child.start_kill();
        }
        if let Some(child) = &mut self.router_child {
            let _ = child.start_kill();
        }
    }
}

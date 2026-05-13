use crate::{artifacts::ArtifactCache, qmp::QmpClient, TestContext};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, trace, warn};

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
        }
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

    pub async fn build(self) -> Result<VirtmcuTestEnv> {
        if self.nodes.is_empty() {
            return Err(anyhow!("Topology must have at least one node"));
        }

        let mut ctx = TestContext::new()?;
        for (k, v) in self.variables {
            ctx.variables.insert(k, v);
        }
        let endpoint = ctx.variables.get("ROUTER_ENDPOINT").unwrap().clone();

        // Spawn a native Zenoh coordinator acting as a router
        let router_bin = ctx.find_binary("zenoh_coordinator")?;

        info!("Spawning zenoh_coordinator from: {}", router_bin.display());

        let mut router_cmd = Command::new(&router_bin);
        router_cmd
            .arg("--listen")
            .arg(&endpoint)
            .arg("--pdes")
            .arg("--nodes")
            .arg(self.nodes.len().to_string());

        let router_proc = router_cmd
            .spawn()
            .map_err(|e| {
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

        tokio::time::sleep(Duration::from_millis(1000)).await;

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
                let yaml_content = std::fs::read_to_string(ctx.workspace_root.join(p))
                    .context(format!("Failed to read YAML path: {}", p))?;

                let yaml_content = yaml_content.replace("ZENOH_ROUTER_ENDPOINT", &endpoint);
                let yaml_content = ctx.substitute(&yaml_content);

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

            qemu_cmd.env("ZENOH_ROUTER_ENDPOINT", &endpoint);
            qemu_cmd.env("VIRTMCU_ZENOH_ROUTER", &endpoint);

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
                qemu_cmd.arg("-device").arg(format!(
                    "virtmcu-transport-hub,id=virtmcu-transport-hub,node={}",
                    node.id
                ));
                qemu_cmd
                    .arg("-global")
                    .arg(format!("virtmcu-transport-hub.router={}", endpoint));

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
                ] {
                    qemu_cmd
                        .arg("-global")
                        .arg(format!("{}.transport=hub0", dev_type));
                }
                let has_clock_in_yaml_args = yaml_cli_args
                    .iter()
                    .any(|arg| arg.contains("virtmcu-clock"));
                if !has_manual_clock && !has_clock_in_yaml_args {
                    qemu_cmd.arg("-device").arg(format!(
                        "virtmcu-clock,mode=slaved-suspend,router={},node={}",
                        endpoint, node.id
                    ));
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
                    {
                        let mut recent = recent_stderr_for_spawn.lock().await;
                        recent.push(line.clone());
                        if recent.len() > 200 {
                            recent.remove(0);
                        }
                    }

                    tracing::debug!("[QEMU] [Node {}] {}", node_id_for_log, line);

                    if line.contains("[ERROR]")
                        || line.contains("error:")
                        || line.contains("fatal:")
                        || line.contains("panic")
                    {
                        error!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[WARN ]") || line.contains("warning:") {
                        warn!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[DEBUG]") {
                        debug!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[TRACE]") {
                        trace!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else if line.contains("[INFO ]") || line.contains("info:") {
                        info!("[QEMU] [Node {}] {}", node_id_for_log, line);
                    } else {
                        info!("[QEMU] [Node {}] {}", node_id_for_log, line);
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
            info!("Liveliness barrier passed. Executing 0-ns VTA sync...");

            let coordinator = ZenohClockCoordinator::new(session.clone());
            coordinator
                .step_clock(0, 0, 0, 0)
                .await
                .map_err(|e| anyhow!("Failed to receive VTA 0-ns sync reply from QEMU: {}", e))?;

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
            _session: session.clone(),
            clock_coordinator: Box::new(ZenohClockCoordinator::new(session)),
            router_child: router_proc,
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
    _session: zenoh::Session,
    clock_coordinator: Box<dyn ClockCoordinator>,
    router_child: Child,
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
        self.qemu_children.push(child);
    }

    pub fn session(&self) -> zenoh::Session {
        self._session.clone()
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
            self.current_quantum += 1;

            for node_id in 0..self.qemu_children.len() {
                if !self.is_coordinated[node_id] {
                    continue;
                }

                self.clock_coordinator
                    .step_clock(node_id, advance, self.current_vtime, self.current_quantum)
                    .await?;
            }
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
                uart_reader.get_mut().read(&mut buf).await.map(|n| (n, buf))
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

        let step_ns: u64 = 10_000_000; // 10ms step

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
                self.current_quantum += 1;

                for node_idx in 0..self.qemu_children.len() {
                    if self.is_coordinated[node_idx] {
                        self.clock_coordinator
                            .step_clock(node_idx, step_ns, self.current_vtime, self.current_quantum)
                            .await?;
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }

            // 2. Try to read from UART
            let uart_reader = &mut self.uart_readers[node_id];
            let uart_read = timeout(Duration::from_millis(5), async {
                use tokio::io::AsyncReadExt;
                let mut buf = [0u8; 1024];
                uart_reader.get_mut().read(&mut buf).await.map(|n| (n, buf))
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
        let _ = self.router_child.kill().await;

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
        let _ = self.router_child.start_kill();
    }
}

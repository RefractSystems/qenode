use anyhow::{anyhow, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use clap::{Parser, Subcommand};
use std::io::Cursor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{error, info};
use virtmcu_test_runner::QmpClient;

use serde_json::Value;
use std::path::PathBuf;

pub mod setup;

#[derive(Parser, Debug)]
#[command(author, version, about = "VirtMCU Debug & Utility CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum QmpCommands {
    /// Show full QOM tree
    Tree,
    /// List immediate properties
    List { path: String },
    /// Get a property value
    Get { path: String, prop: String },
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Subscribe to Zenoh telemetry events
    Telemetry {
        /// Node ID to listen for
        #[arg(default_value = "0")]
        node_id: u32,
        /// Zenoh router endpoint
        #[arg(long)]
        router: Option<String>,
    },
    /// Start a mock MMIO adapter
    FakeAdapter {
        /// Unix socket path to listen on
        #[arg(short, long, default_value = "/tmp/fake_adapter.sock")]
        socket: String,
    },
    /// Inspect QEMU object model via QMP
    Qmp {
        /// Unix socket path of QEMU QMP
        #[arg(short, long, default_value = "qmp.sock")]
        socket: String,
        /// Subcommand to run on QMP
        #[command(subcommand)]
        cmd: QmpCommands,
    },
    /// Workspace and environment setup
    Setup {
        #[command(subcommand)]
        cmd: SetupCommands,
    },
    /// Schema management
    Schema {
        #[command(subcommand)]
        cmd: SchemaCommands,
    },
    /// Platform management (YAML to DTB/CLI)
    Platform {
        #[command(subcommand)]
        cmd: PlatformCommands,
    },
    /// Debugging utilities
    Debug {
        #[command(subcommand)]
        cmd: DebugCommands,
    },
}

#[derive(Subcommand, Debug)]
enum DebugCommands {
    /// Dump Zenoh traffic to a PCAP file
    PcapDump {
        /// Output PCAP file path (use '-' for stdout)
        #[arg(short, long)]
        output: String,
        /// Zenoh router endpoint
        #[arg(short, long)]
        session: Option<String>,
        /// Zenoh topic to subscribe to
        #[arg(short, long, default_value = "sim/coord/**/rx")]
        topic: String,
        /// Subscribe to legacy sim/comm/** topics
        #[arg(long)]
        legacy: bool,
    },
}

#[derive(Subcommand, Debug)]
enum PlatformCommands {
    /// Generate DTB and CLI arguments from a VirtMCU YAML platform description
    Generate {
        /// Input YAML file
        input: PathBuf,
        /// Output DTB file
        #[arg(long)]
        out_dtb: Option<PathBuf>,
        /// Output CLI arguments file
        #[arg(long)]
        out_cli: Option<PathBuf>,
        /// Output architecture name file
        #[arg(long)]
        out_arch: Option<PathBuf>,
        /// Zenoh router endpoint (optional)
        #[arg(long)]
        router: Option<String>,
        /// Node ID (default: 0)
        #[arg(long, default_value = "0")]
        node_id: u32,
    },
    /// Generate C++ address maps from OpenUSD-aligned YAML
    GenerateHeader {
        /// Input YAML file
        input: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum SchemaCommands {
    /// Generate schemas (TypeSpec -> JSON Schema -> Rust/Python)
    Generate,
    /// Verify schemas are up-to-date
    Check,
    /// Generate topic constants from topics.toml
    GenerateTopics,
}

#[derive(Subcommand, Debug)]
enum SetupCommands {
    /// Sync versions
    SyncVersions,
    /// Bootstrap workspace
    Bootstrap,
    /// Clean simulation
    CleanupSim,
    /// Apply virtmcu patches to a QEMU source tree
    PatchQemu {
        /// Path to QEMU source directory
        #[arg(default_value = "third_party/qemu")]
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Telemetry { node_id, router } => run_telemetry(node_id, router).await?,
        Commands::FakeAdapter { socket } => run_fake_adapter(&socket).await?,
        Commands::Qmp { socket, cmd } => run_qmp(&socket, cmd).await?,
        Commands::Setup { cmd } => match cmd {
            SetupCommands::SyncVersions => setup::run_sync_versions().await?,
            SetupCommands::Bootstrap => setup::run_bootstrap().await?,
            SetupCommands::CleanupSim => setup::run_cleanup_sim().await?,
            SetupCommands::PatchQemu { path } => setup::run_patch_qemu(&path).await?,
        },
        Commands::Schema { cmd } => match cmd {
            SchemaCommands::Generate => setup::run_generate_schemas().await?,
            SchemaCommands::Check => setup::run_check_schemas().await?,
            SchemaCommands::GenerateTopics => run_schema_generate_topics().await?,
        },
        Commands::Platform { cmd } => match cmd {
            PlatformCommands::Generate {
                input,
                out_dtb,
                out_cli,
                out_arch,
                router,
                node_id,
            } => run_platform_generate(input, out_dtb, out_cli, out_arch, router, node_id).await?,
            PlatformCommands::GenerateHeader { input } => {
                run_platform_generate_header(input).await?
            }
        },
        Commands::Debug { cmd } => match cmd {
            DebugCommands::PcapDump {
                output,
                session,
                topic,
                legacy,
            } => run_debug_pcap_dump(&output, session, &topic, legacy).await?,
        },
    }

    Ok(())
}

struct PcapDumper {
    writer: Box<dyn std::io::Write + Send>,
}

impl PcapDumper {
    fn new(path: &str) -> Result<Self> {
        let writer: Box<dyn std::io::Write + Send> = if path == "-" {
            Box::new(std::io::stdout())
        } else {
            Box::new(std::fs::File::create(path)?)
        };

        let mut dumper = Self { writer };
        dumper.write_global_header()?;
        Ok(dumper)
    }

    fn write_global_header(&mut self) -> Result<()> {
        use byteorder::WriteBytesExt;
        self.writer.write_u32::<LittleEndian>(0xA1B2C3D4)?; // magic
        self.writer.write_u16::<LittleEndian>(2)?; // version major
        self.writer.write_u16::<LittleEndian>(4)?; // version minor
        self.writer.write_i32::<LittleEndian>(0)?; // thiszone
        self.writer.write_u32::<LittleEndian>(0)?; // sigfigs
        self.writer.write_u32::<LittleEndian>(65535)?; // snaplen
        self.writer.write_u32::<LittleEndian>(147)?; // network (DLT_USER0)
        self.writer.flush()?;
        Ok(())
    }

    fn write_packet(
        &mut self,
        vtime_ns: u64,
        src: u32,
        dst: u32,
        protocol: u16,
        payload: &[u8],
    ) -> Result<()> {
        use byteorder::WriteBytesExt;

        let ts_sec = (vtime_ns / 1_000_000_000) as u32;
        let ts_usec = ((vtime_ns % 1_000_000_000) / 1000) as u32;

        let pcap_proto = match protocol {
            0 => 1,   // Ethernet
            1 => 2,   // UART
            2 => 7,   // SPI
            3 => 4,   // CAN-FD
            4 => 5,   // FlexRay
            5 => 6,   // LIN
            6 => 3,   // IEEE 802.15.4
            7 => 8,   // RF-HCI
            8 => 255, // Control/Test Infra
            _ => 255,
        };

        // DLT_USER0 Header: src(4) + dst(4) + proto(2)
        let incl_len = (10 + payload.len()) as u32;
        let orig_len = incl_len;

        self.writer.write_u32::<LittleEndian>(ts_sec)?;
        self.writer.write_u32::<LittleEndian>(ts_usec)?;
        self.writer.write_u32::<LittleEndian>(incl_len)?;
        self.writer.write_u32::<LittleEndian>(orig_len)?;

        self.writer.write_u32::<LittleEndian>(src)?;
        self.writer.write_u32::<LittleEndian>(dst)?;
        self.writer.write_u16::<LittleEndian>(pcap_proto)?;
        self.writer.write_all(payload)?;
        self.writer.flush()?;

        Ok(())
    }
}

async fn run_debug_pcap_dump(
    output_path: &str,
    router: Option<String>,
    topic_pattern: &str,
    use_legacy: bool,
) -> Result<()> {
    let mut dumper = PcapDumper::new(output_path)?;

    let mut config = zenoh::Config::default();
    let _ = config.insert_json5("scouting/multicast/enabled", "false");
    if let Some(r) = router {
        let _ = config.insert_json5("mode", "\"client\"");
        let _ = config.insert_json5("connect/endpoints", &format!("[\"{}\"]", r));
    }

    let session = zenoh::open(config)
        .await
        .map_err(|e| anyhow!("Zenoh open: {}", e))?;

    let final_topic = if use_legacy && topic_pattern == "sim/coord/**/rx" {
        "sim/comm/**"
    } else {
        topic_pattern
    };

    info!("Starting Zenoh PCAP Dumper...");
    info!("  Topic:   {}", final_topic);
    info!("  Output:  {}", output_path);

    let subscriber = session
        .declare_subscriber(final_topic)
        .await
        .map_err(|e| anyhow!("Zenoh sub: {}", e))?;

    while let Ok(sample) = subscriber.recv_async().await {
        let topic = sample.key_expr().to_string();
        let payload = sample.payload().to_bytes();

        // 1. Try decoding as CoordMessage
        if topic.contains("sim/coord/") {
            if let Ok(msg) = flatbuffers::root::<virtmcu_api::CoordMessage>(&payload) {
                let vtime = msg.delivery_vtime_ns();
                let src = msg.src_node_id();
                let dst = msg.dst_node_id();
                let proto = msg.protocol().0 as u16;
                let data = msg.payload().map(|v| v.bytes()).unwrap_or(&[]);
                dumper.write_packet(vtime, src, dst, proto, data)?;
                continue;
            }
        }

        // 2. Try decoding as Legacy ZenohFrameHeader
        if payload.len() >= virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
            if let Some((header, data)) = virtmcu_api::decode_frame(&payload) {
                let vtime = header.delivery_vtime_ns();
                let mut node_id = 0;
                let mut proto_id = 8; // Default to Control

                let parts: Vec<&str> = topic.split('/').collect();
                for (i, part) in parts.iter().enumerate() {
                    match *part {
                        "eth" => {
                            proto_id = 0;
                            if let Some(n) = parts.get(i + 2) {
                                node_id = n.parse().unwrap_or(0);
                            }
                            break;
                        }
                        "uart" => {
                            proto_id = 1;
                            if let Some(n) = parts.get(i + 1) {
                                node_id = n.parse().unwrap_or(0);
                            }
                            break;
                        }
                        "can" => {
                            proto_id = 3;
                            if let Some(n) = parts.get(i + 1) {
                                node_id = n.parse().unwrap_or(0);
                            }
                            break;
                        }
                        "lin" => {
                            proto_id = 5;
                            if let Some(n) = parts.get(i + 1) {
                                node_id = n.parse().unwrap_or(0);
                            }
                            break;
                        }
                        "spi" => {
                            proto_id = 2;
                            if let Some(n) = parts.get(i + 2) {
                                node_id = n.parse().unwrap_or(0);
                            }
                            break;
                        }
                        p if p.contains("rf") => {
                            proto_id = 6;
                            break;
                        }
                        _ => {}
                    }
                }
                dumper.write_packet(vtime, 0, node_id, proto_id, data)?;
            }
        }
    }

    Ok(())
}

async fn run_platform_generate_header(input: PathBuf) -> Result<()> {
    let content = std::fs::read_to_string(&input)?;
    let data: serde_yaml::Value = serde_yaml::from_str(&content)?;

    println!("/* Generated by virtmcu-cli from {} */", input.display());
    println!("#pragma once\n");
    println!("#include <cstdint>\n");
    println!("namespace virtmcu {{");
    println!("namespace address_map {{\n");

    if let Some(peripherals) = data.get("peripherals").and_then(|p| p.as_sequence()) {
        for periph in peripherals {
            let name = periph
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("UNKNOWN");
            let addr = periph
                .get("address")
                .and_then(|a| {
                    if let Some(s) = a.as_str() {
                        Some(s.to_string())
                    } else if let Some(i) = a.as_u64() {
                        Some(format!("{:#x}", i))
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "0x0".to_string());

            let safe_name = name.replace('-', "_").to_uppercase();
            println!("    constexpr uint64_t {}_BASE = {};", safe_name, addr);
        }
    }

    println!("\n}} // namespace address_map");
    println!("}} // namespace virtmcu");

    Ok(())
}

async fn run_platform_generate(
    input: PathBuf,
    out_dtb: Option<PathBuf>,
    out_cli: Option<PathBuf>,
    out_arch: Option<PathBuf>,
    router: Option<String>,
    node_id: u32,
) -> Result<()> {
    let yaml_content = std::fs::read_to_string(&input)?;
    let (platform, world) = yaml2qemu::parse_yaml(&yaml_content, router.as_deref(), node_id)?;

    if let Some(dtb_path) = out_dtb {
        // We need to compile DTS to DTB. Use dtc.
        let mut child = std::process::Command::new("dtc")
            .args(["-I", "dts", "-O", "dtb", "-o"])
            .arg(&dtb_path)
            .stdin(std::process::Stdio::piped())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        use std::io::Write;
        stdin.write_all(platform.dts_content.as_bytes())?;
        drop(stdin);

        let status = child.wait()?;
        if !status.success() {
            return Err(anyhow!("dtc failed to compile generated DTS"));
        }

        // Validate DTB against world
        yaml2qemu::validate_dtb(&dtb_path, &world)?;
        info!("✓ Generated and validated DTB: {}", dtb_path.display());
    }

    if let Some(cli_path) = out_cli {
        std::fs::write(&cli_path, platform.cli_args.join("\n"))?;
        info!("✓ Generated CLI args: {}", cli_path.display());
    }

    if let Some(arch_path) = out_arch {
        let arch = if let Some(m) = &world.machine {
            if let Some(cpus) = m.cpus.get(0) {
                if cpus.cpu_type.contains("riscv") {
                    "riscv"
                } else {
                    "arm"
                }
            } else {
                "arm"
            }
        } else {
            "arm"
        };
        std::fs::write(&arch_path, arch)?;
        info!("✓ Generated architecture: {}", arch_path.display());
    }

    Ok(())
}

async fn run_telemetry(node_id: u32, router: Option<String>) -> Result<()> {
    let mut config = zenoh::Config::default();
    let _ = config.insert_json5("scouting/multicast/enabled", "false");
    if let Some(r) = router {
        let _ = config.insert_json5("mode", "\"client\"");
        let _ = config.insert_json5("connect/endpoints", &format!("[\"{}\"]", r));
    }

    let session = zenoh::open(config)
        .await
        .map_err(|e| anyhow!("Zenoh open: {}", e))?;
    let topic = format!("sim/telemetry/trace/{}", node_id);
    info!("Listening on {}", topic);

    let subscriber = session
        .declare_subscriber(&topic)
        .await
        .map_err(|e| anyhow!("Zenoh sub: {}", e))?;

    while let Ok(sample) = subscriber.recv_async().await {
        let payload = sample.payload().to_bytes();
        if let Ok(ev) = flatbuffers::root::<
            virtmcu_api::telemetry_generated::virtmcu::telemetry::TraceEvent,
        >(&payload)
        {
            let ts = ev.timestamp_ns();
            let ev_type = ev.type_().0;
            let ev_id = ev.id();
            let val = ev.value();
            let name_str = ev.device_name().unwrap_or("");

            let (type_str, id_str) = match ev_type {
                0 => ("CPU_STATE", format!("cpu={}", ev_id)),
                1 => {
                    let slot = ev_id >> 16;
                    let pin = ev_id & 0xFFFF;
                    let mut s = format!("slot={:2} pin={:2}", slot, pin);
                    if !name_str.is_empty() {
                        s.push_str(&format!(" ({})", name_str));
                    }
                    ("IRQ", s)
                }
                2 => ("PERIPHERAL", format!("id={}", ev_id)),
                _ => ("UNKNOWN", format!("id={}", ev_id)),
            };

            info!("[{:15}] {:10} {} val={:3}", ts, type_str, id_str, val);
        } else {
            error!("Received malformed payload of size {}", payload.len());
        }
    }

    Ok(())
}

async fn run_fake_adapter(socket_path: &str) -> Result<()> {
    if std::path::Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
    }
    let listener = tokio::net::UnixListener::bind(socket_path)?;
    info!("Server listening on {}", socket_path);

    let (mut socket, _) = listener.accept().await?;
    info!("Connected");

    // 12 bytes handshake (VirtmcuHandshake)
    let mut hs_buf = [0u8; 12];
    socket.read_exact(&mut hs_buf).await?;

    // Send back handshake
    socket.write_all(&hs_buf).await?;

    let mut req_buf = [0u8; 32];
    while socket.read_exact(&mut req_buf).await.is_ok() {
        let mut cur = Cursor::new(&req_buf);
        let req_type = ReadBytesExt::read_u32::<LittleEndian>(&mut cur)?;
        let size = ReadBytesExt::read_u32::<LittleEndian>(&mut cur)?;
        let _r1 = ReadBytesExt::read_u32::<LittleEndian>(&mut cur)?;
        let _r2 = ReadBytesExt::read_u32::<LittleEndian>(&mut cur)?;
        let vtime = ReadBytesExt::read_u64::<LittleEndian>(&mut cur)?;
        let addr = ReadBytesExt::read_u64::<LittleEndian>(&mut cur)?;
        let data = ReadBytesExt::read_u64::<LittleEndian>(&mut cur)?;

        info!(
            "REQ: type={}, size={}, vtime={}, addr={:#x}, data={:#x}",
            req_type, size, vtime, addr, data
        );

        let resp = [0u8; 16]; // SyscMsg
        socket.write_all(&resp).await?;
    }

    Ok(())
}

use futures::future::BoxFuture;
use futures::FutureExt;

fn dump_tree<'a>(qmp: &'a mut QmpClient, path: String, depth: usize) -> BoxFuture<'a, Result<()>> {
    async move {
        let res = qmp
            .execute_with_args("qom-list", Some(serde_json::json!({"path": path})))
            .await?;
        if let Some(returns) = res.get("return").and_then(|r| r.as_array()) {
            for item in returns {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    if let Some(type_str) = item.get("type").and_then(|t| t.as_str()) {
                        let indent = "  ".repeat(depth);
                        println!("{}├── {} ({})", indent, name, type_str);
                        if type_str.starts_with("child<") {
                            let child_path = if path == "/" {
                                format!("/{}", name)
                            } else {
                                format!("{}/{}", path, name)
                            };
                            dump_tree(qmp, child_path, depth + 1).await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    .boxed()
}

async fn run_schema_generate_topics() -> Result<()> {
    let toml_path = PathBuf::from("tools/deterministic_coordinator/protocol/topics.toml");
    let content = std::fs::read_to_string(&toml_path)?;
    let config: Value = toml::from_str(&content)?;

    let rs_content = generate_topics_rust(&config);
    let rs_path = PathBuf::from("tools/deterministic_coordinator/src/topics.rs");
    if let Some(parent) = rs_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&rs_path, rs_content)?;
    info!("✓ Generated {}", rs_path.display());

    Ok(())
}

fn generate_topics_rust(config: &Value) -> String {
    let mut lines = vec![
        "// AUTO-GENERATED from topics.toml. DO NOT EDIT MANUALLY.".to_string(),
        "#![allow(dead_code)]".to_string(),
        "".to_string(),
        "pub mod singleton {".to_string(),
    ];

    if let Some(singleton) = config.get("singleton").and_then(|s| s.as_object()) {
        for (name, value) in singleton {
            lines.push(format!(
                "    pub const {}: &str = \"{}\";",
                name,
                value.as_str().unwrap_or("")
            ));
        }
    }
    lines.push("}".to_string());
    lines.push("".to_string());

    lines.push("pub mod wildcard {".to_string());
    if let Some(wildcard) = config.get("wildcard").and_then(|w| w.as_object()) {
        let mut keys: Vec<_> = wildcard.keys().collect();
        keys.sort();
        for name in keys {
            lines.push(format!(
                "    pub const {}: &str = \"{}\";",
                name,
                wildcard.get(name).unwrap().as_str().unwrap_or("")
            ));
        }
    }
    lines.push("}".to_string());
    lines.push("".to_string());

    lines.push("pub const ALL_LEGACY_TX_WILDCARDS: &[&str] = &[".to_string());
    if let Some(wildcard) = config.get("wildcard").and_then(|w| w.as_object()) {
        let mut keys: Vec<_> = wildcard.keys().collect();
        keys.sort();
        for name in keys {
            if name.ends_with("_TX_WILDCARD") && name != "COORD_TX_WILDCARD" {
                lines.push(format!("    wildcard::{},", name));
            }
        }
    }
    lines.push("];".to_string());
    lines.push("".to_string());

    lines.push("pub mod templates {".to_string());
    if let Some(templates) = config.get("templates").and_then(|t| t.as_object()) {
        for (name, value) in templates {
            let val_str = value.as_str().unwrap_or("");
            let val = val_str
                .replace("{node_id}", "{}")
                .replace("{unique_id}", "{}")
                .replace("{plugin}", "{}")
                .replace("{suffix}", "{}")
                .replace("{bus}", "{}")
                .replace("{port_id}", "{}");

            let re = regex::Regex::new(r"\{([a-z_]+)\}").unwrap();
            let placeholders: Vec<_> = re
                .captures_iter(val_str)
                .map(|c| c.get(1).unwrap().as_str())
                .collect();
            let args = placeholders
                .iter()
                .map(|p| format!("{}: &str", p))
                .collect::<Vec<_>>()
                .join(", ");
            let format_args = placeholders.join(", ");
            lines.push(format!("    pub fn {}({}) -> String {{", name, args));
            lines.push(format!("        format!(\"{}\", {})", val, format_args));
            lines.push("    }".to_string());
        }
    }
    lines.push("}".to_string());

    lines.join("\n")
}

async fn run_qmp(socket_path: &str, cmd: QmpCommands) -> Result<()> {
    let mut qmp = QmpClient::connect(std::path::Path::new(socket_path)).await?;

    match cmd {
        QmpCommands::Tree => {
            dump_tree(&mut qmp, "/".to_string(), 0).await?;
        }
        QmpCommands::List { path } => {
            let res = qmp
                .execute_with_args("qom-list", Some(serde_json::json!({"path": path})))
                .await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        QmpCommands::Get { path, prop } => {
            let res = qmp
                .execute_with_args(
                    "qom-get",
                    Some(serde_json::json!({"path": path, "property": prop})),
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
    }
    Ok(())
}

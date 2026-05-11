use anyhow::{anyhow, Result};
use byteorder::{LittleEndian, ReadBytesExt};
use clap::{Parser, Subcommand};
use std::io::Cursor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{error, info};
use virtmcu_test_runner::QmpClient;

#[derive(Parser, Debug)]
#[command(author, version, about = "VirtMCU Debug & Utility CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Telemetry { node_id, router } => run_telemetry(node_id, router).await?,
        Commands::FakeAdapter { socket } => run_fake_adapter(&socket).await?,
        Commands::Qmp { socket, cmd } => run_qmp(&socket, cmd).await?,
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

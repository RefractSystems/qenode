use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};
use cyber_bridge::{
    physics::{NoOpPhysics, PhysicsStep, SharedMemPhysics},
    ZenohTimeAuthorityTransport,
};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use virtmcu_api::{
    ClockAdvanceReq, TimeAuthorityTransport, UnixSocketTimeAuthorityTransport, CLOCK_ERROR_STALL,
};
use zenoh::Wait;

#[derive(Debug, Clone, ValueEnum)]
enum TransportType {
    Zenoh,
    Unix,
}

#[derive(Debug, Clone, ValueEnum)]
enum PhysicsType {
    Noop,
    Shm,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Node to drive
    #[arg(long, default_value_t = 0)]
    node_id: u32,

    /// Quantum size in nanoseconds
    #[arg(long, default_value_t = 1_000_000)]
    delta_ns: u64,

    /// Transport type
    #[arg(long, value_enum, default_value_t = TransportType::Zenoh)]
    transport: TransportType,

    /// Zenoh endpoint or Unix socket path
    #[arg(long)]
    connect: Option<String>,

    /// Physics implementation
    #[arg(long, value_enum, default_value_t = PhysicsType::Noop)]
    physics: PhysicsType,

    /// Number of sensors (for shm physics)
    #[arg(long, default_value_t = 0)]
    n_sensors: u32,

    /// Number of actuators (for shm physics)
    #[arg(long, default_value_t = 0)]
    n_actuators: u32,

    /// Per-quantum timeout in milliseconds
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let transport: Box<dyn TimeAuthorityTransport> = match args.transport {
        TransportType::Zenoh => {
            let mut config = virtmcu_zenoh_config::client_config();
            if let Some(connect) = &args.connect {
                let json_connect = if connect.starts_with('[') && connect.ends_with(']') {
                    connect.clone()
                } else {
                    format!("[\"{connect}\"]")
                };
                config
                    .insert_json5("connect/endpoints", &json_connect)
                    .map_err(|e| anyhow!("Zenoh config error: {e}"))?;
            }
            let session = zenoh::open(config)
                .wait()
                .map_err(|e| anyhow!("Zenoh open failed: {e}"))?;
            Box::new(ZenohTimeAuthorityTransport::new(
                Arc::new(session),
                args.node_id,
            ))
        }
        TransportType::Unix => {
            let path = args
                .connect
                .ok_or_else(|| anyhow!("--connect (path) required for Unix transport"))?;
            Box::new(UnixSocketTimeAuthorityTransport::new(path)?)
        }
    };

    let mut physics: Box<dyn PhysicsStep> = match args.physics {
        PhysicsType::Noop => Box::new(NoOpPhysics),
        PhysicsType::Shm => Box::new(SharedMemPhysics::new(
            args.node_id,
            args.n_sensors,
            args.n_actuators,
        )?),
    };

    let mut quantum_number: u64 = 0;
    let mut absolute_vtime_ns: u64 = 0;
    let timeout = Duration::from_millis(args.timeout_ms);

    info!("Starting Time Authority for node {}", args.node_id);

    loop {
        let req = ClockAdvanceReq::new(args.delta_ns, absolute_vtime_ns, quantum_number);

        let resp = transport.advance(req, timeout).ok_or_else(|| {
            error!("Transport timeout or error at quantum {}", quantum_number);
            anyhow!("Transport timeout or error at quantum {}", quantum_number)
        })?;

        if resp.error_code() == CLOCK_ERROR_STALL {
            error!(
                "Clock stall detected at quantum {}. Simulation aborted.",
                quantum_number
            );
            return Err(anyhow!(
                "Clock stall detected at quantum {}. Simulation aborted.",
                quantum_number
            ));
        }

        physics.step(args.delta_ns, &resp)?;

        quantum_number += 1;
        absolute_vtime_ns += args.delta_ns;
    }
}

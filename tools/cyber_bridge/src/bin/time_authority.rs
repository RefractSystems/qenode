use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};
use cyber_bridge::{
    physics::{GatewayPhysics, NoOpPhysics, PhysicsStep, SharedMemPhysics},
    physics_transport::{UnixSocketPhysicsTransport, ZenohPhysicsTransport},
    ZenohActuatorSink, ZenohTimeAuthorityTransport,
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
    Gateway,
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

    /// Zenoh endpoint for data (sensors/actuators) when using Unix transport
    #[arg(long)]
    data_connect: Option<String>,

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

    /// Actuator topic prefix to subscribe
    #[arg(long, default_value = "firmware/control")]
    topic_prefix: String,

    /// Sensor topic prefix to publish
    #[arg(long, default_value = "sim/sensor")]
    sensor_prefix: String,

    /// Physics Gateway transport type
    #[arg(long, value_enum, default_value_t = TransportType::Unix)]
    gateway_transport: TransportType,

    /// Physics Gateway endpoint
    #[arg(long)]
    gateway_connect: Option<String>,
}

async fn open_zenoh_session(connect: Option<&String>) -> Result<Arc<zenoh::Session>> {
    let mut config = virtmcu_zenoh_config::client_config();
    if let Some(connect) = connect {
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
    Ok(Arc::new(session))
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
        "virtmcu-time-authority",
        std::sync::Arc::new(DummyVTimeProvider),
    );
    let args = Args::parse();

    let mut zenoh_session: Option<Arc<zenoh::Session>> = None;

    let transport: Box<dyn TimeAuthorityTransport> = match args.transport {
        TransportType::Zenoh => {
            let session = open_zenoh_session(args.connect.as_ref()).await?;
            zenoh_session = Some(Arc::clone(&session));
            Box::new(ZenohTimeAuthorityTransport::new(session, args.node_id))
        }
        TransportType::Unix => {
            let path = args
                .connect
                .as_ref()
                .ok_or_else(|| anyhow!("--connect (path) required for Unix transport"))?;
            Box::new(UnixSocketTimeAuthorityTransport::new(path)?)
        }
    };

    // If we don't have a zenoh session yet but need one for data
    if zenoh_session.is_none()
        && (args.data_connect.is_some() || matches!(args.physics, PhysicsType::Shm))
    {
        zenoh_session = Some(open_zenoh_session(args.data_connect.as_ref()).await?);
    }

    let actuator_sink = if let Some(session) = &zenoh_session {
        Some(ZenohActuatorSink::new(session, &args.topic_prefix, args.node_id).await?)
    } else {
        None
    };

    let mut physics: Box<dyn PhysicsStep> = match args.physics {
        PhysicsType::Noop => Box::new(NoOpPhysics),
        PhysicsType::Shm => {
            let session = zenoh_session
                .ok_or_else(|| anyhow!("Zenoh session required for SHM physics (sensors)"))?;
            Box::new(SharedMemPhysics::new(
                args.node_id,
                args.n_sensors,
                args.n_actuators,
                session,
                args.sensor_prefix.clone(),
                args.timeout_ms,
            )?)
        }
        PhysicsType::Gateway => {
            let transport: Box<dyn virtmcu_api::PhysicsGatewayTransport> =
                match args.gateway_transport {
                    TransportType::Unix => {
                        let path = args.gateway_connect.as_ref().ok_or_else(|| {
                            anyhow!("--gateway-connect (path) required for Unix gateway transport")
                        })?;
                        Box::new(UnixSocketPhysicsTransport::new(path))
                    }
                    TransportType::Zenoh => {
                        let session = if let Some(session) = &zenoh_session {
                            Arc::clone(session)
                        } else {
                            open_zenoh_session(args.gateway_connect.as_ref()).await?
                        };
                        Box::new(ZenohPhysicsTransport::new(session))
                    }
                };
            Box::new(GatewayPhysics::new(transport, args.timeout_ms))
        }
    };

    let mut quantum_number: u64 = 0;
    let mut absolute_vtime_ns: u64 = 0;
    let timeout = Duration::from_millis(args.timeout_ms);

    info!("Starting Time Authority for node {}", args.node_id);

    loop {
        let req = ClockAdvanceReq::new(args.delta_ns, absolute_vtime_ns, quantum_number);

        let mut resp_opt = None;
        if quantum_number == 0 {
            for i in 1..=60 {
                if let Some(resp) = transport.advance(req, timeout) {
                    resp_opt = Some(resp);
                    break;
                }
                tracing::warn!(
                    "No response at quantum 0 (attempt {}). Retrying in 1s...",
                    i
                );
                std::thread::sleep(Duration::from_secs(1));
            }
        } else {
            resp_opt = transport.advance(req, timeout);
        }

        let resp = resp_opt.ok_or_else(|| {
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

        let actuators: std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>> =
            actuator_sink
                .as_ref()
                .map(|s| s.drain())
                .unwrap_or_default();
        physics.step(args.delta_ns, &actuators, &resp)?;

        quantum_number += 1;
        absolute_vtime_ns += args.delta_ns;
    }
}

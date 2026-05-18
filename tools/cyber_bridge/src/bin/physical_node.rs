#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};
use cyber_bridge::{
    physics::{EmbeddedPlant, RemotePlant, TickOnlyPlant},
    physics_transport::{UnixSocketPhysicsTransport, ZenohPhysicsTransport},
    ZenohActuatorSink, ZenohPhysicalNodeTransport,
};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use virtmcu_wire::{
    ClockAdvanceReq, PhysicalNodeTransport, UnixSocketPhysicalNodeTransport, CLOCK_ERROR_STALL,
};
use zenoh::Wait;

#[derive(Debug, Clone, ValueEnum)]
enum TransportType {
    Zenoh,
    Unix,
}

#[derive(Debug, Clone, ValueEnum)]
enum PlantType {
    /// Advance virtual time only; no physics dynamics.
    TickOnly,
    /// In-process SHM plant: write actuators to /dev/shm, read sensors back.
    Embedded,
    /// Delegate to an external Physics Gateway process via transport.
    Remote,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Identifier for this running simulation instance (HLA: federation name).
    /// Used in log output and Zenoh session metadata. Required.
    #[arg(long, env = "VIRTMCU_FEDERATION_ID")]
    federation_id: String,

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

    /// Physical plant implementation
    #[arg(long, value_enum, default_value_t = PlantType::TickOnly)]
    plant: PlantType,

    /// Number of sensors
    #[arg(long, default_value_t = 0)]
    n_sensors: u32,

    /// Number of actuators
    #[arg(long, default_value_t = 0)]
    n_actuators: u32,

    /// Per-quantum timeout in milliseconds
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,

    /// Actuator topic prefix to subscribe
    #[arg(long, default_value = "firmware/control")]
    topic_prefix: String,

    /// Topic prefix for sensor publications (default: sim/sensor).
    #[arg(long, default_value = "sim/sensor")]
    sensor_prefix: String,

    /// Physics Gateway transport type
    #[arg(long, value_enum, default_value_t = TransportType::Unix)]
    gateway_transport: TransportType,

    /// Physics Gateway endpoint
    #[arg(long)]
    gateway_connect: Option<String>,
}

async fn open_zenoh_session(
    connect: Option<&String>,
    federation_id: &str,
) -> Result<Arc<zenoh::Session>> {
    let mut config = virtmcu_zenoh_config::client_config();

    let _ = config.insert_json5("metadata/federation_id", &format!("\"{}\"", federation_id));

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

#[derive(Debug)]
struct DummyVTimeProvider;
impl virtmcu_observability::processors::VTimeProvider for DummyVTimeProvider {
    fn current_vtime_ns(&self) -> u64 {
        0
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _telemetry = virtmcu_observability::init_telemetry(
        "virtmcu-physical-node",
        std::sync::Arc::new(DummyVTimeProvider),
    );
    let args = Args::parse();
    let federation_id = virtmcu_wire::FederationId(args.federation_id.clone());

    let mut zenoh_session: Option<Arc<zenoh::Session>> = None;

    let transport: Box<dyn PhysicalNodeTransport> = match args.transport {
        TransportType::Zenoh => {
            let session = open_zenoh_session(args.connect.as_ref(), federation_id.as_str()).await?;
            zenoh_session = Some(Arc::clone(&session));
            Box::new(ZenohPhysicalNodeTransport::new(session, args.node_id))
        }
        TransportType::Unix => {
            let path = args
                .connect
                .as_ref()
                .ok_or_else(|| anyhow!("--connect (path) required for Unix transport"))?;
            Box::new(UnixSocketPhysicalNodeTransport::new(path)?)
        }
    };

    // If we don't have a zenoh session yet but need one for data
    if zenoh_session.is_none()
        && (args.data_connect.is_some() || matches!(args.plant, PlantType::Embedded))
    {
        zenoh_session =
            Some(open_zenoh_session(args.data_connect.as_ref(), federation_id.as_str()).await?);
    }

    let actuator_sink = if let Some(session) = &zenoh_session {
        Some(ZenohActuatorSink::new(session, &args.node_id.to_string()).await?)
    } else {
        None
    };

    let mut plant: Box<dyn virtmcu_wire::PhysicalNode> = match args.plant {
        PlantType::TickOnly => Box::new(TickOnlyPlant),
        PlantType::Embedded => Box::new(EmbeddedPlant::new(
            args.node_id,
            args.n_sensors,
            args.n_actuators,
            args.timeout_ms,
        )?),
        PlantType::Remote => {
            let transport: Box<dyn virtmcu_wire::PhysicsGatewayTransport> = match args
                .gateway_transport
            {
                TransportType::Unix => {
                    let path = args.gateway_connect.as_ref().ok_or_else(|| {
                        anyhow!("--gateway-connect required for Remote plant with Unix transport")
                    })?;
                    Box::new(UnixSocketPhysicsTransport::new(path))
                }
                TransportType::Zenoh => {
                    let session = if let Some(ref s) = zenoh_session {
                        Arc::clone(s)
                    } else {
                        open_zenoh_session(args.gateway_connect.as_ref(), federation_id.as_str())
                            .await?
                    };
                    Box::new(ZenohPhysicsTransport::new(session))
                }
            };
            Box::new(RemotePlant::new(transport, args.timeout_ms))
        }
    };

    let mut quantum_number: u64 = 0;
    let mut absolute_vtime_ns: u64 = 0;
    let timeout = Duration::from_millis(args.timeout_ms);

    info!(
        federation = %federation_id,
        plant = ?args.plant,
        node_id = args.node_id,
        delta_ns = args.delta_ns,
        "Starting Physical Node"
    );

    loop {
        tracing::info!(
            federation = %federation_id,
            quantum = quantum_number,
            "quantum start"
        );
        let req = ClockAdvanceReq::new(args.delta_ns, absolute_vtime_ns, quantum_number);

        let mut resp_opt = None;
        if quantum_number == 0 {
            for i in 1..=60 {
                if let Some(resp) = transport.advance(req, timeout) {
                    resp_opt = Some(resp);
                    break;
                }
                tracing::debug!(
                    "No response at quantum 0 (attempt {}). Retrying in 1s...",
                    i
                );
                std::thread::sleep(std::time::Duration::from_secs(1));
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

        let actuators = actuator_sink
            .as_ref()
            .map(|s| s.drain())
            .unwrap_or_default();

        let plant_state = plant
            .step(args.delta_ns, &actuators)
            .map_err(|e| anyhow!("Plant step failed at quantum {quantum_number}: {e}"))?;

        // Publish sensors returned by in-process plant (EmbeddedPlant)
        if let Some(ref session) = zenoh_session {
            for (&sensor_id, vals) in &plant_state.sensors {
                let topic = format!(
                    "{}/{}/sensordata_{}",
                    args.sensor_prefix, args.node_id, sensor_id
                );
                let mut bytes: Vec<u8> = Vec::with_capacity(vals.len() * 8);
                for &v in vals {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                let vtime_end = absolute_vtime_ns + args.delta_ns;
                let payload = virtmcu_wire::encode_frame(vtime_end, 0, &bytes);
                session
                    .put(&topic, payload)
                    .wait()
                    .map_err(|e| anyhow!("Sensor publish failed: {e}"))?;
            }
        }

        quantum_number += 1;
        absolute_vtime_ns += args.delta_ns;
    }
}

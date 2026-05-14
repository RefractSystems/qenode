#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
use anyhow::{anyhow, Result};
use clap::Parser;
use cyber_bridge::physics::{
    SHM_DATA_OFFSET, SHM_HEADER_SIZE, SHM_OFF_BRIDGE_SEQ, SHM_OFF_N_ACTUATORS, SHM_OFF_N_SENSORS,
    SHM_OFF_PHYSICS_SEQ, SHM_OFF_RESERVED, SHM_OFF_SHUTDOWN,
};
use cyber_bridge::physics_transport::{UnixSocketPhysicsServer, ZenohPhysicsServer};
use memmap2::MmapMut;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use virtmcu_api::physics_proto;
use virtmcu_api::PhysicsGatewayServer;
use zenoh::Wait;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Identifier for this running simulation instance (HLA: federation name).
    /// Used in log output. Required.
    #[arg(long, env = "VIRTMCU_FEDERATION_ID")]
    federation_id: String,

    /// Transport type
    #[arg(long, default_value = "unix")]
    transport: String,

    /// Zenoh endpoint or Unix socket path for control
    #[arg(long)]
    connect: String,

    /// identifies the SHM file
    #[arg(long, default_value_t = 0)]
    node_id: u32,

    /// Number of sensors
    #[arg(long, default_value_t = 1)]
    n_sensors: u32,

    /// Number of actuators
    #[arg(long, default_value_t = 1)]
    n_actuators: u32,

    /// Per-quantum timeout in milliseconds
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,

    /// Zenoh endpoint for publishing sensor data. Optional; omit to skip sensor publishing.
    #[arg(long)]
    data_connect: Option<String>,

    /// Topic prefix for sensor publications (default: sim/sensor).
    #[arg(long, default_value = "sim/sensor")]
    sensor_prefix: String,
}

struct GatewayShm {
    mmap: MmapMut,
    shm_path: std::path::PathBuf,
    n_sensors: u32,
    n_actuators: u32,
}

impl GatewayShm {
    fn new(node_id: u32, n_sensors: u32, n_actuators: u32) -> Result<Self> {
        if n_sensors == 0 && n_actuators == 0 {
            anyhow::bail!("GatewayShm requires at least 1 sensor or 1 actuator. Failing loudly.");
        }

        let shm_path = std::path::PathBuf::from(format!("/dev/shm/virtmcu_physics_{node_id}"));
        let size = SHM_HEADER_SIZE + (n_sensors + n_actuators) as usize * 8;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&shm_path)?;

        file.set_len(size as u64)?;
        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        // Write header
        mmap[SHM_OFF_N_SENSORS..SHM_OFF_N_SENSORS + 4].copy_from_slice(&n_sensors.to_le_bytes());
        mmap[SHM_OFF_N_ACTUATORS..SHM_OFF_N_ACTUATORS + 4]
            .copy_from_slice(&n_actuators.to_le_bytes());
        mmap[SHM_OFF_BRIDGE_SEQ..SHM_OFF_BRIDGE_SEQ + 4].copy_from_slice(&0u32.to_le_bytes());
        mmap[SHM_OFF_PHYSICS_SEQ..SHM_OFF_PHYSICS_SEQ + 4].copy_from_slice(&0u32.to_le_bytes());
        mmap[SHM_OFF_SHUTDOWN..SHM_OFF_SHUTDOWN + 4].copy_from_slice(&0u32.to_le_bytes());
        mmap[SHM_OFF_RESERVED..SHM_OFF_RESERVED + 4].copy_from_slice(&0u32.to_le_bytes());

        Ok(Self {
            mmap,
            shm_path,
            n_sensors,
            n_actuators,
        })
    }

    pub fn sensor_bytes(&self, sensor_index: u32) -> &[u8] {
        let offset = SHM_DATA_OFFSET + (sensor_index as usize) * 8;
        &self.mmap[offset..offset + 8]
    }

    fn step(&mut self, trigger: physics_proto::PhysicsTrigger<'_>, timeout_ms: u64) -> Result<()> {
        let quantum_end = trigger.quantum_end_vtime_ns();

        // 1. Extract last actuator value per ID from trigger
        let mut latest_actuators = HashMap::new();
        if let Some(actuators) = trigger.actuators() {
            for sample in actuators {
                let id = sample.actuator_id();
                if let Some(vals) = sample.values() {
                    let v: Vec<f64> = vals.iter().collect();
                    latest_actuators.insert(id, v);
                }
            }
        }

        // 2. Write to SHM
        let ctrl_offset = SHM_DATA_OFFSET + (self.n_sensors as usize) * 8;
        for i in 0..self.n_actuators {
            let val = latest_actuators
                .get(&i)
                .and_then(|v| v.first())
                .copied()
                .expect("Invalid data format");
            let offset = ctrl_offset + (i as usize) * 8;
            self.mmap[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        }

        // 3. Increment bridge_seq and wake via futex
        let bridge_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_BRIDGE_SEQ) as *const AtomicU32;
        let current_bridge_seq = unsafe { (*bridge_ptr).load(Ordering::Acquire) };
        let next_bridge_seq = current_bridge_seq.wrapping_add(1);
        unsafe { (*bridge_ptr).store(next_bridge_seq, Ordering::Release) };

        #[cfg(target_os = "linux")]
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                bridge_ptr,
                libc::FUTEX_WAKE,
                1i32,
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0i32,
            );
        }

        // 4. Wait for physics_seq == bridge_seq
        let physics_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_PHYSICS_SEQ) as *const AtomicU32;
        let start = std::time::Instant::now();
        loop {
            let current_physics_seq = unsafe { (*physics_ptr).load(Ordering::Acquire) };
            if current_physics_seq == next_bridge_seq {
                break;
            }
            if start.elapsed().as_millis() > timeout_ms as u128 {
                return Err(anyhow!("Physics engine timeout at vtime {}ns", quantum_end));
            }

            #[cfg(target_os = "linux")]
            {
                let ts = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000,
                };
                unsafe {
                    libc::syscall(
                        libc::SYS_futex,
                        physics_ptr,
                        libc::FUTEX_WAIT,
                        current_physics_seq,
                        &ts as *const libc::timespec,
                        std::ptr::null::<u32>(),
                        0i32,
                    );
                }
            }
            #[cfg(not(target_os = "linux"))]
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Ok(())
    }

    fn request_shutdown(&mut self) {
        let shutdown_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_SHUTDOWN) as *mut u32;
        unsafe { *shutdown_ptr = 1u32.to_le() };

        let bridge_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_BRIDGE_SEQ) as *const AtomicU32;
        let seq = unsafe { (*bridge_ptr).load(Ordering::Acquire) }.wrapping_add(1);
        unsafe { (*bridge_ptr).store(seq, Ordering::Release) };

        #[cfg(target_os = "linux")]
        unsafe {
            libc::syscall(libc::SYS_futex, bridge_ptr, libc::FUTEX_WAKE, 1i32);
        }
    }
}

impl Drop for GatewayShm {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.shm_path);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let federation_id = virtmcu_api::FederationId(args.federation_id.clone());
    let timeout = std::time::Duration::from_millis(args.timeout_ms);

    let mut shm = GatewayShm::new(args.node_id, args.n_sensors, args.n_actuators)?;

    let server: Box<dyn PhysicsGatewayServer> = match args.transport.as_str() {
        "unix" => Box::new(UnixSocketPhysicsServer::new(&args.connect)?),
        "zenoh" => {
            let mut config = virtmcu_zenoh_config::client_config();
            let _ = config.insert_json5(
                "metadata/federation_id",
                &format!("\"{}\"", federation_id.as_str()),
            );
            let json_connect = format!("[\"{}\"]", args.connect);
            config
                .insert_json5("connect/endpoints", &json_connect)
                .map_err(|e| anyhow!("Zenoh config error: {e}"))?;
            let session = Arc::new(zenoh::open(config).wait().map_err(|e| anyhow!("{e}"))?);
            Box::new(ZenohPhysicsServer::new(session))
        }
        _ => return Err(anyhow!("Unknown transport: {}", args.transport)),
    };

    let zenoh_session: Option<Arc<zenoh::Session>> = if let Some(ref endpoint) = args.data_connect {
        let mut config = virtmcu_zenoh_config::client_config();
        let _ = config.insert_json5(
            "metadata/federation_id",
            &format!("\"{}\"", federation_id.as_str()),
        );
        let json_connect = format!("[\"{endpoint}\"]");
        config
            .insert_json5("connect/endpoints", &json_connect)
            .map_err(|e| anyhow::anyhow!("Zenoh config error: {e}"))?;
        Some(Arc::new(
            zenoh::open(config)
                .wait()
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        ))
    } else {
        None
    };

    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = Arc::clone(&running);
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).unwrap();
        let mut sigterm = signal(SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }
        r.store(false, Ordering::SeqCst);
    });

    while running.load(Ordering::SeqCst) {
        if let Some(trigger_bytes) = server.recv_trigger(timeout) {
            let trigger = flatbuffers::root::<physics_proto::PhysicsTrigger>(&trigger_bytes)?;
            let quantum_number = trigger.quantum_number();

            tracing::info!(
                federation = %federation_id,
                quantum = quantum_number,
                vtime_ns = trigger.quantum_end_vtime_ns(),
                "PhysicsDone ack"
            );

            let res = shm.step(trigger, args.timeout_ms);
            let status = if res.is_ok() { 0 } else { 1 };

            if res.is_ok() {
                if let Some(ref session) = zenoh_session {
                    for i in 0..args.n_sensors {
                        let val_bytes = shm.sensor_bytes(i);
                        let topic =
                            format!("{}/{}/sensordata_{}", args.sensor_prefix, args.node_id, i);
                        let payload =
                            virtmcu_api::encode_frame(trigger.quantum_end_vtime_ns(), 0, val_bytes);
                        session
                            .put(&topic, payload)
                            .wait()
                            .map_err(|e| anyhow::anyhow!("Zenoh sensor publish failed: {e}"))?;
                    }
                }
            }

            let done = physics_proto::PhysicsDone::new(quantum_number, status, 0);
            server.send_done(done).map_err(|e| anyhow!(e))?;

            res?;
        }
    }

    shm.request_shutdown();
    Ok(())
}

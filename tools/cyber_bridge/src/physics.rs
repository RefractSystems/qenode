use anyhow::Result;
use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::sync::Arc;
use virtmcu_api::ClockReadyResp;
use zenoh::Wait;

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU32, Ordering};

/// Final SHM header layout (24 bytes, all little-endian):
///
/// [0..4]   n_sensors:   u32  — number of sensor f64 slots
/// [4..8]   n_actuators: u32  — number of actuator/ctrl f64 slots
/// [8..12]  bridge_seq:  u32  — incremented by gateway to wake physics engine
/// [12..16] physics_seq: u32  — incremented by physics engine to wake gateway
/// [16..20] shutdown:    u32  — set to 1 by gateway to request clean exit
/// [20..24] reserved:    u32  — must be zero; reserved for future use
/// [24..]   data:              — n_sensors f64s, then n_actuators f64s
pub const SHM_OFF_N_SENSORS: usize = 0;
pub const SHM_OFF_N_ACTUATORS: usize = 4;
pub const SHM_OFF_BRIDGE_SEQ: usize = 8;
pub const SHM_OFF_PHYSICS_SEQ: usize = 12;
pub const SHM_OFF_SHUTDOWN: usize = 16;
pub const SHM_OFF_RESERVED: usize = 20;
pub const SHM_DATA_OFFSET: usize = 24;
pub const SHM_HEADER_SIZE: usize = 24;

/// Trait for physics engine integration.
pub trait PhysicsStep: Send + Sync {
    /// Advance physics by delta_ns.
    fn step(
        &mut self,
        delta_ns: u64,
        actuators: &std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>>,
        resp: &ClockReadyResp,
    ) -> Result<()>;
}

/// A physics implementation that does nothing.
pub struct NoOpPhysics;

impl PhysicsStep for NoOpPhysics {
    fn step(
        &mut self,
        _delta_ns: u64,
        _actuators: &std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>>,
        _resp: &ClockReadyResp,
    ) -> Result<()> {
        Ok(())
    }
}

/// A physics implementation that delegates to a remote gateway.
pub struct GatewayPhysics {
    transport: Box<dyn virtmcu_api::PhysicsGatewayTransport>,
    timeout: std::time::Duration,
}

impl GatewayPhysics {
    /// Creates a new `GatewayPhysics` instance.
    pub fn new(transport: Box<dyn virtmcu_api::PhysicsGatewayTransport>, timeout_ms: u64) -> Self {
        Self {
            transport,
            timeout: std::time::Duration::from_millis(timeout_ms),
        }
    }
}

impl PhysicsStep for GatewayPhysics {
    fn step(
        &mut self,
        _delta_ns: u64,
        actuators: &std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>>,
        resp: &ClockReadyResp,
    ) -> Result<()> {
        let mut builder = flatbuffers::FlatBufferBuilder::with_capacity(1024);

        let mut samples = Vec::new();
        for (&vtime, id_map) in actuators {
            for (&id, vals) in id_map {
                let v_offset = builder.create_vector(vals);
                let sample = virtmcu_api::physics_proto::ActuatorSample::create(
                    &mut builder,
                    &virtmcu_api::physics_proto::ActuatorSampleArgs {
                        delivery_vtime_ns: vtime,
                        actuator_id: id,
                        values: Some(v_offset),
                    },
                );
                samples.push(sample);
            }
        }

        let samples_offset = builder.create_vector(&samples);
        let trigger = virtmcu_api::physics_proto::PhysicsTrigger::create(
            &mut builder,
            &virtmcu_api::physics_proto::PhysicsTriggerArgs {
                quantum_number: resp.quantum_number(),
                quantum_end_vtime_ns: resp.current_vtime_ns(),
                actuators: Some(samples_offset),
            },
        );

        builder.finish(trigger, None);
        let trigger_bytes = builder.finished_data();

        self.transport
            .trigger_and_wait(trigger_bytes, self.timeout)
            .map_err(|e| anyhow::anyhow!("Physics Gateway trigger failed: {e}"))?;

        Ok(())
    }
}

/// A physics implementation that communicates via shared memory.
pub struct SharedMemPhysics {
    mmap: MmapMut,
    shm_path: std::path::PathBuf,
    node_id: u32,
    n_sensors: u32,
    n_actuators: u32,
    session: Arc<zenoh::Session>,
    topic_prefix: String,
    timeout_ms: u64,
    bridge_seq: u32,
}

impl SharedMemPhysics {
    /// Creates a new `SharedMemPhysics` instance.
    pub fn new(
        node_id: u32,
        n_sensors: u32,
        n_actuators: u32,
        session: Arc<zenoh::Session>,
        topic_prefix: String,
        timeout_ms: u64,
    ) -> Result<Self> {
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

        // Write header — all fields little-endian u32
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
            node_id,
            n_sensors,
            n_actuators,
            session,
            topic_prefix,
            timeout_ms,
            bridge_seq: 0,
        })
    }
}

impl Drop for SharedMemPhysics {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.shm_path) {
            // Log but do not panic — Drop must not unwind
            eprintln!(
                "SharedMemPhysics: failed to remove {}: {e}",
                self.shm_path.display()
            );
        }
    }
}

impl PhysicsStep for SharedMemPhysics {
    fn step(
        &mut self,
        delta_ns: u64,
        actuators: &std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>>,
        resp: &ClockReadyResp,
    ) -> Result<()> {
        let quantum_end = resp.current_vtime_ns();
        let quantum_start = quantum_end.saturating_sub(delta_ns);

        // For each actuator slot, use the LAST command issued within this quantum.
        // Multiple writes to the same actuator in one quantum: last value wins.
        let mut quantum_actuators: std::collections::HashMap<u32, Vec<f64>> =
            std::collections::HashMap::new();
        for (_vtime, id_map) in actuators.range(quantum_start..quantum_end) {
            for (&id, vals) in id_map {
                quantum_actuators.insert(id, vals.clone());
            }
        }

        let ctrl_offset = SHM_DATA_OFFSET + (self.n_sensors as usize) * 8;

        // 1. Write actuator (ctrl) values to SHM
        for i in 0..self.n_actuators {
            let val = quantum_actuators
                .get(&i)
                .and_then(|v| v.first())
                .copied()
                .unwrap_or(0.0);
            let offset = ctrl_offset + (i as usize) * 8;
            self.mmap[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        }

        // 2. Increment bridge_seq and wake the physics engine via futex
        self.bridge_seq = self.bridge_seq.wrapping_add(1);
        let bridge_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_BRIDGE_SEQ) as *const AtomicU32;
        unsafe { (*bridge_ptr).store(self.bridge_seq, Ordering::Release) };

        #[cfg(target_os = "linux")]
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                bridge_ptr,
                libc::FUTEX_WAKE,
                1i32, // wake at most 1 waiter
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0i32,
            );
        }

        // 3. Wait for physics engine to increment physics_seq via futex
        let physics_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_PHYSICS_SEQ) as *const AtomicU32;
        let expected = self.bridge_seq;
        let start = std::time::Instant::now();

        loop {
            let current = unsafe { (*physics_ptr).load(Ordering::Acquire) };
            if current == expected {
                break;
            }
            if start.elapsed().as_millis() > self.timeout_ms as u128 {
                return Err(anyhow::anyhow!(
                    "Physics engine timeout at vtime {}ns",
                    resp.current_vtime_ns()
                ));
            }

            #[cfg(target_os = "linux")]
            {
                let ts = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000, // 10 ms kernel timeout
                };
                let ret = unsafe {
                    libc::syscall(
                        libc::SYS_futex,
                        physics_ptr,
                        libc::FUTEX_WAIT,
                        current, // only sleep if *ptr still == current
                        &ts as *const libc::timespec,
                        std::ptr::null::<u32>(),
                        0i32,
                    )
                };
                if ret == -1 {
                    let err = unsafe { *libc::__errno_location() };
                    match err {
                        libc::EAGAIN | libc::EINTR => continue, // value changed or signal, retry
                        libc::ETIMEDOUT => continue,            // kernel timeout, check wall-clock
                        _ => return Err(anyhow::anyhow!("futex WAIT error: errno {err}")),
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // 4. Read sensor values from SHM and publish to Zenoh
        for i in 0..self.n_sensors {
            let offset = SHM_DATA_OFFSET + (i as usize) * 8;
            let val_bytes = &self.mmap[offset..offset + 8];
            let topic = format!("{}/{}/sensordata_{}", self.topic_prefix, self.node_id, i);
            let payload = virtmcu_api::encode_frame(resp.current_vtime_ns(), 0, val_bytes);
            self.session
                .put(&topic, payload)
                .wait()
                .map_err(|e| anyhow::anyhow!("Zenoh publish failed: {e}"))?;
        }

        Ok(())
    }
}

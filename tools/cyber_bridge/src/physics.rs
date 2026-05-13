use anyhow::Result;
use memmap2::MmapMut;
use std::fs::OpenOptions;

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

/// A physical plant that advances virtual time but models no dynamics.
///
/// Use for pure cyber-node testing and nodes with no actuators or sensors.
pub struct TickOnlyPlant;

impl virtmcu_api::PhysicalNode for TickOnlyPlant {
    fn step(
        &mut self,
        _quantum_ns: u64,
        _actuators: &virtmcu_api::ActuatorMap,
    ) -> Result<virtmcu_api::PlantState, String> {
        Ok(virtmcu_api::PlantState {
            vtime_ns: 0, // caller tracks absolute vtime; this field unused for TickOnly
            sensors: virtmcu_api::SensorMap::new(),
        })
    }
}

/// A physical plant that delegates dynamics to an external Physics Gateway process.
///
/// Sends a `PhysicsTrigger` FlatBuffer to the gateway and blocks until `PhysicsDone`
/// is received. The gateway publishes sensor data directly to Zenoh; this struct
/// returns an empty `sensors` map.
pub struct RemotePlant {
    transport: Box<dyn virtmcu_api::PhysicsGatewayTransport>,
    timeout: std::time::Duration,
    quantum_number: u64, // tracked here so the trigger matches ClockAdvanceReq
}

impl RemotePlant {
    /// Creates a new `RemotePlant`.
    pub fn new(transport: Box<dyn virtmcu_api::PhysicsGatewayTransport>, timeout_ms: u64) -> Self {
        Self {
            transport,
            timeout: std::time::Duration::from_millis(timeout_ms),
            quantum_number: 0,
        }
    }
}

impl virtmcu_api::PhysicalNode for RemotePlant {
    fn step(
        &mut self,
        _quantum_ns: u64,
        actuators: &virtmcu_api::ActuatorMap,
    ) -> Result<virtmcu_api::PlantState, String> {
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
                quantum_number: self.quantum_number,
                quantum_end_vtime_ns: 0, // binary tracks vtime; field unused by gateway for RemotePlant
                actuators: Some(samples_offset),
            },
        );

        builder.finish(trigger, None);
        let trigger_bytes = builder.finished_data();

        self.transport
            .trigger_and_wait(trigger_bytes, self.timeout)?;

        self.quantum_number += 1;

        Ok(virtmcu_api::PlantState {
            vtime_ns: 0,
            sensors: virtmcu_api::SensorMap::new(),
        })
    }
}

/// A physical plant that communicates via shared memory.
pub struct EmbeddedPlant {
    mmap: MmapMut,
    shm_path: std::path::PathBuf,
    _node_id: u32,
    n_sensors: u32,
    n_actuators: u32,
    timeout_ms: u64,
    bridge_seq: u32,
}

impl EmbeddedPlant {
    /// Creates a new `EmbeddedPlant` instance.
    pub fn new(node_id: u32, n_sensors: u32, n_actuators: u32, timeout_ms: u64) -> Result<Self> {
        if n_sensors == 0 && n_actuators == 0 {
            anyhow::bail!(
                "EmbeddedPlant requires at least 1 sensor or 1 actuator. Failing loudly."
            );
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
            _node_id: node_id,
            n_sensors,
            n_actuators,
            timeout_ms,
            bridge_seq: 0,
        })
    }
}

impl Drop for EmbeddedPlant {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.shm_path) {
            // Log but do not panic — Drop must not unwind
            eprintln!(
                "EmbeddedPlant: failed to remove {}: {e}",
                self.shm_path.display()
            );
        }
    }
}

impl virtmcu_api::PhysicalNode for EmbeddedPlant {
    fn step(
        &mut self,
        _quantum_ns: u64,
        actuators: &virtmcu_api::ActuatorMap,
    ) -> Result<virtmcu_api::PlantState, String> {
        // EmbeddedPlant takes the last value per actuator_id from the full map:
        let mut ctrl_values: std::collections::BTreeMap<u32, f64> =
            std::collections::BTreeMap::new();
        for id_map in actuators.values() {
            for (&id, vals) in id_map {
                if let Some(&v) = vals.first() {
                    ctrl_values.insert(id, v);
                }
            }
        }

        let ctrl_offset = SHM_DATA_OFFSET + (self.n_sensors as usize) * 8;

        // 1. Write actuator (ctrl) values to SHM
        for i in 0..self.n_actuators {
            let val = ctrl_values.get(&i).copied().unwrap_or(0.0);
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
                return Err(format!(
                    "Physics engine timeout for bridge_seq {}",
                    self.bridge_seq
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
                        _ => return Err(format!("futex WAIT error: errno {err}")),
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // 4. Read sensor values from SHM and return them in PlantState
        let mut sensors = virtmcu_api::SensorMap::new();
        for i in 0..self.n_sensors {
            let offset = SHM_DATA_OFFSET + (i as usize) * 8;
            let val = f64::from_le_bytes(
                self.mmap[offset..offset + 8]
                    .try_into()
                    .expect("SHM sensor slice is 8 bytes"),
            );
            sensors.insert(i, vec![val]);
        }
        Ok(virtmcu_api::PlantState {
            vtime_ns: 0, // binary tracks vtime; field unused by main loop for EmbeddedPlant
            sensors,
        })
    }
}

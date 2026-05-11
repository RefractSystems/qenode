use anyhow::Result;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use virtmcu_api::ClockReadyResp;

/// Trait for physics engine integration.
pub trait PhysicsStep: Send + Sync {
    /// Advance physics by delta_ns.
    fn step(&mut self, delta_ns: u64, resp: &ClockReadyResp) -> Result<()>;
}

/// A physics implementation that does nothing.
pub struct NoOpPhysics;

impl PhysicsStep for NoOpPhysics {
    fn step(&mut self, _delta_ns: u64, _resp: &ClockReadyResp) -> Result<()> {
        Ok(())
    }
}

/// A physics implementation that communicates via shared memory.
pub struct SharedMemPhysics {
    file: std::fs::File,
    _size: usize,
    _n_sensors: u32,
    _n_actuators: u32,
}

impl SharedMemPhysics {
    /// Creates a new `SharedMemPhysics` instance.
    pub fn new(node_id: u32, n_sensors: u32, n_actuators: u32) -> Result<Self> {
        let shm_name = format!("/dev/shm/virtmcu_mujoco_{node_id}");
        let size = 16 + (n_sensors + n_actuators) as usize * 8;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o666)
            .open(&shm_name)?;

        file.set_len(size as u64)?;

        Ok(Self {
            file,
            _size: size,
            _n_sensors: n_sensors,
            _n_actuators: n_actuators,
        })
    }
}

impl PhysicsStep for SharedMemPhysics {
    fn step(&mut self, _delta_ns: u64, resp: &ClockReadyResp) -> Result<()> {
        // Write the header to shared memory to signal activity
        self.file.seek(SeekFrom::Start(0))?;
        let mut header = [0u8; 16];
        header[0..8].copy_from_slice(&resp.current_vtime_ns().to_le_bytes());
        header[8..16].copy_from_slice(&resp.quantum_number().to_le_bytes());
        self.file.write_all(&header)?;
        Ok(())
    }
}

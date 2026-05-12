use anyhow::{anyhow, Result};
use memmap2::MmapMut;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use virtmcu_api::ClockReadyResp;
use zenoh::Wait;

/// Trait for physics engine integration.
pub trait PhysicsStep: Send + Sync {
    /// Advance physics by delta_ns.
    fn step(
        &mut self,
        delta_ns: u64,
        actuators: &HashMap<u32, Vec<f64>>,
        resp: &ClockReadyResp,
    ) -> Result<()>;
}

/// A physics implementation that does nothing.
pub struct NoOpPhysics;

impl PhysicsStep for NoOpPhysics {
    fn step(
        &mut self,
        _delta_ns: u64,
        _actuators: &HashMap<u32, Vec<f64>>,
        _resp: &ClockReadyResp,
    ) -> Result<()> {
        Ok(())
    }
}

/// A physics implementation that communicates via shared memory.
pub struct SharedMemPhysics {
    mmap: MmapMut,
    node_id: u32,
    n_sensors: u32,
    n_actuators: u32,
    session: Arc<zenoh::Session>,
    topic_prefix: String,
    timeout_ms: u64,
    bridge_seq: u64,
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
        let shm_name = format!("/dev/shm/virtmcu_mujoco_{node_id}");
        let size = 24 + (n_sensors + n_actuators) as usize * 8;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&shm_name)?;

        file.set_len(size as u64)?;

        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        // Write header
        mmap[0..4].copy_from_slice(&n_sensors.to_le_bytes());
        mmap[4..8].copy_from_slice(&n_actuators.to_le_bytes());

        // Initialize sequences to 0
        let bridge_seq_ptr = mmap.as_ptr().wrapping_add(8) as *const AtomicU64;
        let mujoco_seq_ptr = mmap.as_ptr().wrapping_add(16) as *const AtomicU64;
        unsafe {
            (*bridge_seq_ptr).store(0, Ordering::SeqCst);
            (*mujoco_seq_ptr).store(0, Ordering::SeqCst);
        }

        Ok(Self {
            mmap,
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

impl PhysicsStep for SharedMemPhysics {
    fn step(
        &mut self,
        _delta_ns: u64,
        actuators: &HashMap<u32, Vec<f64>>,
        resp: &ClockReadyResp,
    ) -> Result<()> {
        let ctrl_offset = 24 + (self.n_sensors as usize) * 8;

        // 1. Write ctrl[] to shm
        for i in 0..self.n_actuators {
            let val = actuators
                .get(&i)
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or(0.0);
            let offset = ctrl_offset + (i as usize) * 8;
            self.mmap[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        }

        // 2. Atomically increment bridge_seq
        self.bridge_seq += 1;
        let bridge_seq_ptr = self.mmap.as_ptr().wrapping_add(8) as *const AtomicU64;
        unsafe {
            (*bridge_seq_ptr).store(self.bridge_seq, Ordering::SeqCst);
        }

        // 3. Poll mujoco_seq
        let mujoco_seq_ptr = self.mmap.as_ptr().wrapping_add(16) as *const AtomicU64;
        let start = std::time::Instant::now();
        loop {
            let mujoco_seq = unsafe { (*mujoco_seq_ptr).load(Ordering::SeqCst) };
            if mujoco_seq == self.bridge_seq {
                break;
            }
            if start.elapsed().as_millis() > self.timeout_ms as u128 {
                return Err(anyhow!(
                    "MuJoCo bridge timeout at vtime {}",
                    resp.current_vtime_ns()
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // 4. Read sensordata[] and publish to Zenoh
        for i in 0..self.n_sensors {
            let offset = 24 + (i as usize) * 8;
            let val_bytes = &self.mmap[offset..offset + 8];

            let topic = format!("{}/{}/sensordata_{}", self.topic_prefix, self.node_id, i);
            let payload = virtmcu_api::encode_frame(resp.current_vtime_ns(), 0, val_bytes);

            self.session
                .put(&topic, payload)
                .wait()
                .map_err(|e| anyhow!("Zenoh publish failed: {}", e))?;
        }

        Ok(())
    }
}

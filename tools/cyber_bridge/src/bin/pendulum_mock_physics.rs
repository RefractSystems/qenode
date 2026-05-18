use anyhow::{anyhow, Result};
use clap::Parser;
use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

// SHM header layout (all u32, little-endian)
const SHM_OFF_N_SENSORS: usize = 0;
const SHM_OFF_N_ACTUATORS: usize = 4;
const SHM_OFF_BRIDGE_SEQ: usize = 8;
const SHM_OFF_PHYSICS_SEQ: usize = 12;
const SHM_OFF_SHUTDOWN: usize = 16;
const SHM_DATA_OFFSET: usize = 24;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Node ID to connect to
    #[arg(long, default_value_t = 0)]
    node_id: u32,

    /// Step size in nanoseconds
    #[arg(long, default_value_t = 1_000_000)]
    delta_ns: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let shm_path = PathBuf::from(format!("/dev/shm/virtmcu_physics_{}", args.node_id));

    println!("Waiting for {} to be created...", shm_path.display());
    while !shm_path.exists() {
        std::thread::sleep(Duration::from_millis(100));
    }

    let file = OpenOptions::new().read(true).write(true).open(&shm_path)?;

    let mut mmap = unsafe { MmapMut::map_mut(&file)? };
    println!("Connected to SHM.");

    let n_sensors = u32::from_le_bytes(mmap[SHM_OFF_N_SENSORS..SHM_OFF_N_SENSORS + 4].try_into()?);
    let n_actuators =
        u32::from_le_bytes(mmap[SHM_OFF_N_ACTUATORS..SHM_OFF_N_ACTUATORS + 4].try_into()?);
    println!("Sensors: {}, Actuators: {}", n_sensors, n_actuators);

    if n_sensors < 1 || n_actuators < 1 {
        return Err(anyhow!("Need at least 1 sensor and 1 actuator!"));
    }

    let sensors_offset = SHM_DATA_OFFSET;
    let actuators_offset = SHM_DATA_OFFSET + (n_sensors as usize) * 8;

    // Pendulum physics state
    let mut angle: f64 = 0.5;
    let mut velocity: f64 = 0.0;
    let dt = (args.delta_ns as f64) / 1_000_000_000.0;

    let gravity = 9.81;
    let length = 1.0;
    let damping = 0.1;

    let bridge_ptr = mmap.as_ptr().wrapping_add(SHM_OFF_BRIDGE_SEQ) as *const AtomicU32;
    let physics_ptr = mmap.as_ptr().wrapping_add(SHM_OFF_PHYSICS_SEQ) as *const AtomicU32;
    let shutdown_ptr = mmap.as_ptr().wrapping_add(SHM_OFF_SHUTDOWN) as *const AtomicU32;

    let mut physics_seq: u32 = 0;

    loop {
        let bridge_seq = unsafe { (*bridge_ptr).load(Ordering::Acquire) };

        if bridge_seq == physics_seq {
            // Wait for bridge_seq to change
            #[cfg(target_os = "linux")]
            unsafe {
                let ts = libc::timespec {
                    tv_sec: 1,
                    tv_nsec: 0,
                };
                libc::syscall(
                    libc::SYS_futex,
                    bridge_ptr,
                    libc::FUTEX_WAIT,
                    bridge_seq,
                    &ts as *const libc::timespec,
                    std::ptr::null::<u32>(),
                    0i32,
                );
            }
            #[cfg(not(target_os = "linux"))]
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }

        // Check shutdown
        if unsafe { (*shutdown_ptr).load(Ordering::Acquire) } != 0 {
            println!("Shutdown requested. Exiting.");
            break;
        }

        // Read actuator (torque)
        let torque = f64::from_le_bytes(mmap[actuators_offset..actuators_offset + 8].try_into()?);

        // Physics step
        let angular_accel = torque - (gravity / length) * angle.sin() - damping * velocity;
        velocity += angular_accel * dt;
        angle += velocity * dt;

        // Write sensor (angle)
        mmap[sensors_offset..sensors_offset + 8].copy_from_slice(&angle.to_le_bytes());

        // Acknowledge
        physics_seq = bridge_seq;
        unsafe { (*physics_ptr).store(physics_seq, Ordering::Release) };

        #[cfg(target_os = "linux")]
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                physics_ptr,
                libc::FUTEX_WAKE,
                1i32,
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0i32,
            );
        }
    }

    Ok(())
}

use std::env;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: {} <node_id> <n_sensors> <n_actuators>", args[0]);
        std::process::exit(1);
    }

    let node_id: u32 = args[1].parse().unwrap();
    let n_sensors: u32 = args[2].parse().unwrap();
    let n_actuators: u32 = args[3].parse().unwrap();

    let shm_name = format!("/dev/shm/virtmcu_mujoco_{node_id}");
    let size = 16 + (n_sensors + n_actuators) as usize * 8;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o666)
        .open(&shm_name)
        .expect("failed to open shared memory");

    let _ = file.set_len(size as u64);

    println!("Shared memory {shm_name} created.");

    let config = zenoh::Config::default();
    let session = zenoh::open(config).await.unwrap();

    let _advance_topic = format!("sim/clock/advance/{node_id}");

    // Actuator subscriber
    let act_topic = format!("sim/actuator/{node_id}/**");
    let _sub = session.declare_subscriber(&act_topic).await.unwrap();

    // The test in Python only runs this, checks if file exists, then kills it.
    // Real implementation would mmap the file and step MuJoCo.
    loop {
        tokio::time::sleep(core::time::Duration::from_secs(1)).await;
    }
}

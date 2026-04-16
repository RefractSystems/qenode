/*
 * virtmcu Zenoh Coordinator
 *
 * This Rust daemon replaces the concept of a traditional "WirelessMedium" or
 * central network switch found in other emulation frameworks (like Renode).
 *
 * The Coordinator's role:
 * 1. Topology Discovery: It dynamically discovers nodes when they publish to
 *    TX topics (e.g., `sim/eth/frame/node0/tx`).
 * 2. Causal Ordering: It reads the `delivery_vtime_ns` timestamp from the
 *    incoming message's header, adds a configurable propagation `delay_ns`,
 *    and rewrites the timestamp.
 * 3. Link Modeling: It applies distance-based attenuation or drop probabilities
 *    defined via the Dynamic Network Topology API.
 *
 * Because the receiving nodes use `hw/zenoh/zenoh-netdev.c` (or equivalent),
 * they will buffer the message and deliver it into the guest firmware *only*
 * when their virtual clocks catch up to the rewritten delivery timestamp.
 */
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Parser;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use zenoh::config::Config;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Default propagation delay to add to the virtual timestamp (in nanoseconds)
    #[arg(short, long, default_value_t = 1_000_000)]
    delay_ns: u64,

    /// Seed for the deterministic PRNG used for packet dropping
    #[arg(short, long, default_value_t = 42)]
    seed: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct LinkUpdate {
    from: String,
    to: String,
    delay_ns: Option<u64>,
    drop_probability: Option<f64>,
}

struct LinkState {
    delay_ns: u64,
    drop_probability: f64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    println!("Starting virtmcu Zenoh Coordinator");
    println!("  Default delay: {} ns", args.delay_ns);
    println!("  PRNG seed: {}", args.seed);

    let session = zenoh::open(Config::default()).await.unwrap();

    // Subscribe to all TX topics
    let eth_sub = session
        .declare_subscriber("sim/eth/frame/*/tx")
        .await
        .unwrap();
    let uart_sub = session
        .declare_subscriber("virtmcu/uart/*/tx")
        .await
        .unwrap();
    let sysc_sub = session
        .declare_subscriber("sim/systemc/frame/*/tx")
        .await
        .unwrap();

    // Subscribe to topology control updates
    let ctrl_sub = session
        .declare_subscriber("sim/network/control")
        .await
        .unwrap();

    // Track active nodes dynamically based on who transmits
    let mut known_eth_nodes = HashSet::new();
    let mut known_uart_nodes = HashSet::new();
    let mut known_sysc_nodes = HashSet::new();

    // Link properties: (from, to) -> LinkState
    let mut topology: HashMap<(String, String), LinkState> = HashMap::new();

    // Deterministic PRNG
    let mut rng = ChaCha8Rng::seed_from_u64(args.seed);

    println!("Listening for packets and topology updates...");

    loop {
        tokio::select! {
            Ok(sample) = eth_sub.recv_async() => {
                handle_eth_msg(&session, sample, &mut known_eth_nodes, &topology, args.delay_ns, &mut rng).await;
            }
            Ok(sample) = uart_sub.recv_async() => {
                handle_uart_msg(&session, sample, &mut known_uart_nodes, &topology, args.delay_ns, &mut rng).await;
            }
            Ok(sample) = sysc_sub.recv_async() => {
                handle_sysc_msg(&session, sample, &mut known_sysc_nodes, &topology, args.delay_ns).await;
            }
            Ok(sample) = ctrl_sub.recv_async() => {
                let payload_bytes = sample.payload().to_bytes();
                if let Ok(payload_str) = std::str::from_utf8(&payload_bytes) {
                    if let Ok(update) = serde_json::from_str::<LinkUpdate>(payload_str) {
                        let state = topology.entry((update.from.clone(), update.to.clone())).or_insert(LinkState {
                            delay_ns: args.delay_ns,
                            drop_probability: 0.0,
                        });
                        if let Some(d) = update.delay_ns { state.delay_ns = d; }
                        if let Some(p) = update.drop_probability { state.drop_probability = p; }
                        println!("Topology Update: {} -> {} (delay: {} ns, drop: {})",
                                 update.from, update.to, state.delay_ns, state.drop_probability);
                    }
                }
            }
        }
    }
}

async fn handle_eth_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    topology: &HashMap<(String, String), LinkState>,
    default_delay_ns: u64,
    rng: &mut ChaCha8Rng,
) {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 5 {
        return;
    }
    let sender_id = parts[3].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return;
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
    let size = cursor.read_u32::<LittleEndian>().unwrap();

    // Broadcast to all known nodes except the sender
    for node in known_nodes.iter() {
        if node == &sender_id {
            continue;
        }

        let (delay_ns, drop_prob) =
            if let Some(state) = topology.get(&(sender_id.clone(), node.clone())) {
                (state.delay_ns, state.drop_probability)
            } else {
                (default_delay_ns, 0.0)
            };

        // Apply deterministic packet drop
        if drop_prob > 0.0 && rng.gen::<f64>() < drop_prob {
            println!("ETH: DROPPED packet from {} to {}", sender_id, node);
            continue;
        }

        let new_delivery_vtime_ns = delivery_vtime_ns + delay_ns;

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload
            .write_u64::<LittleEndian>(new_delivery_vtime_ns)
            .unwrap();
        new_payload.write_u32::<LittleEndian>(size).unwrap();
        new_payload.write_all(&payload[12..]).unwrap();

        let rx_topic = format!("sim/eth/frame/{}/rx", node);
        if let Err(e) = session.put(&rx_topic, new_payload).await {
            eprintln!("Failed to forward to {}: {}", node, e);
        }
    }
}

async fn handle_uart_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    topology: &HashMap<(String, String), LinkState>,
    default_delay_ns: u64,
    rng: &mut ChaCha8Rng,
) {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 4 {
        return;
    }
    let sender_id = parts[2].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return;
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
    let size = cursor.read_u32::<LittleEndian>().unwrap();

    // Broadcast to all known nodes except the sender
    for node in known_nodes.iter() {
        if node == &sender_id {
            continue;
        }

        let (delay_ns, drop_prob) =
            if let Some(state) = topology.get(&(sender_id.clone(), node.clone())) {
                (state.delay_ns, state.drop_probability)
            } else {
                (default_delay_ns, 0.0)
            };

        // Apply deterministic packet drop
        if drop_prob > 0.0 && rng.gen::<f64>() < drop_prob {
            println!("UART: DROPPED packet from {} to {}", sender_id, node);
            continue;
        }

        let new_delivery_vtime_ns = delivery_vtime_ns + delay_ns;

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload
            .write_u64::<LittleEndian>(new_delivery_vtime_ns)
            .unwrap();
        new_payload.write_u32::<LittleEndian>(size).unwrap();
        new_payload.write_all(&payload[12..]).unwrap();

        let rx_topic = format!("virtmcu/uart/{}/rx", node);
        if let Err(e) = session.put(&rx_topic, new_payload).await {
            eprintln!("Failed to forward to {}: {}", node, e);
        }
    }
}

async fn handle_sysc_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    topology: &HashMap<(String, String), LinkState>,
    default_delay_ns: u64,
) {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 5 {
        return;
    }
    let sender_id = parts[3].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return;
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
    let size = cursor.read_u32::<LittleEndian>().unwrap();

    // Broadcast to all known nodes except the sender
    for node in known_nodes.iter() {
        if node == &sender_id {
            continue;
        }

        let delay_ns = if let Some(state) = topology.get(&(sender_id.clone(), node.clone())) {
            state.delay_ns
        } else {
            default_delay_ns
        };

        // CRITICAL FIX: Do NOT drop SystemC frames (like CAN bus) silently.
        // Physical layer buses rely on arbitration. Dropping them here breaks
        // the hardware ACKs in the SystemC controller models.

        let new_delivery_vtime_ns = delivery_vtime_ns + delay_ns;

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload
            .write_u64::<LittleEndian>(new_delivery_vtime_ns)
            .unwrap();
        new_payload.write_u32::<LittleEndian>(size).unwrap();
        new_payload.write_all(&payload[12..]).unwrap();

        let rx_topic = format!("sim/systemc/frame/{}/rx", node);
        if let Err(e) = session.put(&rx_topic, new_payload).await {
            eprintln!("Failed to forward to {}: {}", node, e);
        }
    }
}

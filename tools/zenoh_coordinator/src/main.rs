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

    /// TX power in dBm for RF simulations
    #[arg(short, long, default_value_t = 0.0)]
    tx_power: f32,

    /// RX sensitivity in dBm. Packets below this will be dropped.
    #[arg(short, long, default_value_t = -90.0)]
    sensitivity: f32,
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

struct NodeInfo {
    id: String,
    x: f64,
    y: f64,
    z: f64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    println!("Starting virtmcu Zenoh Coordinator");
    println!("  Default delay: {} ns", args.delay_ns);
    println!("  PRNG seed: {}", args.seed);
    println!("  RF TX Power: {} dBm", args.tx_power);
    println!("  RF Sensitivity: {} dBm", args.sensitivity);

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
    let rf_802154_sub = session
        .declare_subscriber("sim/rf/802154/*/tx")
        .await
        .unwrap();
    let rf_hci_sub = session
        .declare_subscriber("sim/rf/hci/*/tx")
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
    let mut known_rf_nodes = HashSet::new();

    // Link properties: (from, to) -> LinkState
    let mut topology: HashMap<(String, String), LinkState> = HashMap::new();

    // Mock node positions (In a real system, these would come from a physics engine via Zenoh)
    let mut node_positions = HashMap::new();
    node_positions.insert("0".to_string(), NodeInfo { id: "0".to_string(), x: 0.0, y: 0.0, z: 0.0 });
    node_positions.insert("1".to_string(), NodeInfo { id: "1".to_string(), x: 10.0, y: 0.0, z: 0.0 });
    node_positions.insert("2".to_string(), NodeInfo { id: "2".to_string(), x: 100.0, y: 0.0, z: 0.0 });

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
            Ok(sample) = rf_802154_sub.recv_async() => {
                handle_rf_msg(&session, sample, &mut known_rf_nodes, "sim/rf/802154", &node_positions, &args, true).await;
            }
            Ok(sample) = rf_hci_sub.recv_async() => {
                handle_rf_msg(&session, sample, &mut known_rf_nodes, "sim/rf/hci", &node_positions, &args, false).await;
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

async fn handle_rf_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    topic_prefix: &str,
    positions: &HashMap<String, NodeInfo>,
    args: &Args,
    has_rf_header: bool,
) {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    let sender_id = parts[parts.len() - 2].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    let header_size = if has_rf_header { 14 } else { 12 };
    if payload.len() < header_size {
        return;
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
    let size = cursor.read_u32::<LittleEndian>().unwrap();

    let sender_pos = positions.get(&sender_id);

    for receiver_id in known_nodes.iter() {
        if receiver_id == &sender_id { continue; }

        let receiver_pos = positions.get(receiver_id);
        
        let mut rssi = args.tx_power;
        let mut extra_delay_ns = args.delay_ns;

        if let (Some(s), Some(r)) = (sender_pos, receiver_pos) {
            let dist = ((s.x - r.x).powi(2) + (s.y - r.y).powi(2) + (s.z - r.z).powi(2)).sqrt();
            if dist.is_normal() || dist == 0.0 {
                let path_loss = calculate_fspl(dist, 2.4e9);
                if !path_loss.is_nan() {
                    rssi -= path_loss as f32;
                }
                
                // Speed of light delay: ~3.33 ns per meter
                let dist_delay = (dist * 3.33) as u64;
                extra_delay_ns = extra_delay_ns.saturating_add(dist_delay);
            }
        }

        if rssi < args.sensitivity {
            println!("RF: Drop packet from {} to {} (RSSI: {:.1} dBm < {:.1} dBm)", 
                     sender_id, receiver_id, rssi, args.sensitivity);
            continue;
        }

        let new_delivery_vtime_ns = delivery_vtime_ns.saturating_add(extra_delay_ns);
        assert!(new_delivery_vtime_ns >= delivery_vtime_ns, "Virtual time must not move backwards (overflow detected)");

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns).unwrap();
        new_payload.write_u32::<LittleEndian>(size).unwrap();
        
        if has_rf_header {
            new_payload.write_i8(rssi as i8).unwrap();
            new_payload.write_u8(255).unwrap(); // LQI
            new_payload.write_all(&payload[14..]).unwrap();
        } else {
            new_payload.write_all(&payload[12..]).unwrap();
        }

        let rx_topic = format!("{}/{}/rx", topic_prefix, receiver_id);
        let _ = session.put(&rx_topic, new_payload).await;
        
        println!("RF: Forwarded from {} to {} (vtime+{}ns, RSSI: {:.1} dBm)", 
                 sender_id, receiver_id, extra_delay_ns, rssi);
    }
}

fn calculate_fspl(dist_m: f64, freq_hz: f64) -> f64 {
    if dist_m < 0.1 { return 0.0; }
    let c = 299_792_458.0;
    // FSPL (dB) = 20 log10(d) + 20 log10(f) + 20 log10(4π/c)
    20.0 * dist_m.log10() + 20.0 * freq_hz.log10() + 20.0 * (4.0 * std::f64::consts::PI / c).log10()
}

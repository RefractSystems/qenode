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
 */
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Parser;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use std::sync::Arc;
use tokio::sync::RwLock;
use virtmcu_api::rf_generated::rf_header;
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
    #[arg(long, default_value_t = -90.0)]
    sensitivity: f32,

    /// Path to YAML file defining AABB obstacles with per-box dB attenuation.
    /// Format: obstacles: [{x_min, x_max, y_min, y_max, z_min, z_max, attenuation_db}]
    #[arg(long)]
    obstacles: Option<String>,
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

/// Node position sourced from physics engine via sim/telemetry/position.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct NodeInfo {
    id: String,
    x: f64,
    y: f64,
    z: f64,
}

/// Position update message published by MuJoCo / physics engine.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct PositionUpdate {
    id: String,
    x: f64,
    y: f64,
    z: f64,
}

// ─── Obstacle Attenuation Model (Phase 14.9) ─────────────────────────────────
//
// Each obstacle is an axis-aligned bounding box (AABB) with a fixed dB
// attenuation penalty.  For every TX→RX pair the coordinator checks whether
// the line segment between sender and receiver intersects any obstacle using
// the parametric slab method.  All intersecting obstacles' attenuation values
// are summed and subtracted from the computed RSSI before the sensitivity
// check, so thick walls and stacked obstacles combine additively.
//
// Loaded from a YAML file at startup via --obstacles <path>.

/// Single AABB obstacle entry loaded from the YAML config.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ObstacleBox {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    z_min: f64,
    z_max: f64,
    /// Signal attenuation in dB when line-of-sight passes through this obstacle.
    attenuation_db: f64,
}

/// Top-level YAML structure for obstacle config files.
#[derive(Serialize, Deserialize, Debug, Default)]
struct ObstaclesConfig {
    obstacles: Vec<ObstacleBox>,
}

/// Slab method: returns true iff the segment from (ox,oy,oz) to (tx,ty,tz)
/// intersects the given AABB.  Uses parametric t ∈ [0, 1] for the segment.
fn ray_intersects_aabb(ox: f64, oy: f64, oz: f64, tx: f64, ty: f64, tz: f64, obs: &ObstacleBox) -> bool {
    let (dx, dy, dz) = (tx - ox, ty - oy, tz - oz);
    let mut t_min = 0.0_f64;
    let mut t_max = 1.0_f64;

    for (b_min, b_max, d, o) in [
        (obs.x_min, obs.x_max, dx, ox),
        (obs.y_min, obs.y_max, dy, oy),
        (obs.z_min, obs.z_max, dz, oz),
    ] {
        if d.abs() < f64::EPSILON {
            if o < b_min || o > b_max {
                return false;
            }
        } else {
            let t1 = (b_min - o) / d;
            let t2 = (b_max - o) / d;
            t_min = t_min.max(t1.min(t2));
            t_max = t_max.min(t1.max(t2));
            if t_min > t_max {
                return false;
            }
        }
    }
    t_min <= t_max
}
// ─────────────────────────────────────────────────────────────────────────────

// ─── Spatial Grid (Phase 14.6) ────────────────────────────────────────────────
//
// Partitions 3D space into cubic cells of size CELL_SIZE_M.  For each RF TX
// packet, only nodes in the sender's cell and its 26 immediate neighbours are
// checked for reception.  This reduces the per-packet work from O(N) to
// O(nodes_in_27_cells) — typically O(1) for sparse simulations and still
// sub-linear for dense ones.
//
// Assumption: node positions change slowly relative to the quantum rate.
// The grid is rebuilt lazily on every position update (write path) so the
// read path (per-packet routing) holds no locks.

/// Cell edge length in metres.  Nodes more than 1.5 × CELL_SIZE_M apart
/// cannot be in adjacent cells, so this value acts as the maximum RF range
/// for the spatial index.  The FSPL model still drops packets that are below
/// the sensitivity threshold, so this is a conservative upper bound.
const CELL_SIZE_M: f64 = 500.0;

/// 3D integer cell coordinate.
type CellKey = (i64, i64, i64);

fn world_to_cell(x: f64, y: f64, z: f64) -> CellKey {
    (
        (x / CELL_SIZE_M).floor() as i64,
        (y / CELL_SIZE_M).floor() as i64,
        (z / CELL_SIZE_M).floor() as i64,
    )
}

/// Spatial grid: cell → list of node IDs.
struct SpatialGrid {
    cells: HashMap<CellKey, Vec<String>>,
}

impl SpatialGrid {
    fn build(positions: &HashMap<String, NodeInfo>) -> Self {
        let mut cells: HashMap<CellKey, Vec<String>> = HashMap::new();
        for info in positions.values() {
            let key = world_to_cell(info.x, info.y, info.z);
            cells.entry(key).or_default().push(info.id.clone());
        }
        SpatialGrid { cells }
    }

    /// Return node IDs in the same cell and all 26 immediate neighbours of
    /// the given position, excluding `sender_id`.
    fn candidates(&self, x: f64, y: f64, z: f64, sender_id: &str) -> Vec<String> {
        let (cx, cy, cz) = world_to_cell(x, y, z);
        let mut out = Vec::new();
        for dx in -1i64..=1 {
            for dy in -1i64..=1 {
                for dz in -1i64..=1 {
                    let key = (cx + dx, cy + dy, cz + dz);
                    if let Some(ids) = self.cells.get(&key) {
                        for id in ids {
                            if id != sender_id {
                                out.push(id.clone());
                            }
                        }
                    }
                }
            }
        }
        out
    }
}
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = Args::parse();
    println!("Starting virtmcu Zenoh Coordinator");
    println!("  Default delay: {} ns", args.delay_ns);
    println!("  PRNG seed: {}", args.seed);
    println!("  RF TX Power: {} dBm", args.tx_power);
    println!("  RF Sensitivity: {} dBm", args.sensitivity);

    let obstacles: Vec<ObstacleBox> = if let Some(ref path) = args.obstacles {
        let file = std::fs::File::open(path)
            .unwrap_or_else(|e| panic!("Cannot open obstacles file '{}': {}", path, e));
        let config: ObstaclesConfig = serde_yaml::from_reader(file)
            .unwrap_or_else(|e| panic!("Failed to parse obstacles YAML '{}': {}", path, e));
        println!("  Obstacles: {} box(es) loaded from '{}'", config.obstacles.len(), path);
        config.obstacles
    } else {
        Vec::new()
    };

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

    // Subscribe to dynamic position updates from the physics engine (MuJoCo).
    // Format: JSON {"id":"0","x":1.0,"y":2.0,"z":3.0} published at physics rate (~1 kHz).
    // Protected by RwLock: read lock during RF routing, write lock only during updates.
    let pos_sub = session
        .declare_subscriber("sim/telemetry/position")
        .await
        .unwrap();

    // Track active nodes dynamically based on who transmits
    let mut known_eth_nodes = HashSet::new();
    let mut known_uart_nodes = HashSet::new();
    let mut known_sysc_nodes = HashSet::new();
    let mut known_rf_nodes = HashSet::new();

    // Link properties: (from, to) -> LinkState
    let mut topology: HashMap<(String, String), LinkState> = HashMap::new();

    // Node positions: dynamically updated via sim/telemetry/position.
    // Seed with default positions; physics engine updates will override these.
    let node_positions: Arc<RwLock<HashMap<String, NodeInfo>>> = {
        let mut m = HashMap::new();
        m.insert("0".to_string(), NodeInfo { id: "0".to_string(), x: 0.0, y: 0.0, z: 0.0 });
        m.insert("1".to_string(), NodeInfo { id: "1".to_string(), x: 10.0, y: 0.0, z: 0.0 });
        m.insert("2".to_string(), NodeInfo { id: "2".to_string(), x: 100.0, y: 0.0, z: 0.0 });
        Arc::new(RwLock::new(m))
    };

    // Deterministic PRNG
    let mut rng = ChaCha8Rng::seed_from_u64(args.seed);

    println!("Listening for packets and topology updates...");

    loop {
        tokio::select! {
            Ok(sample) = eth_sub.recv_async() => {
                let _ = handle_eth_msg(&session, sample, &mut known_eth_nodes, &topology, args.delay_ns, &mut rng).await;
            }
            Ok(sample) = uart_sub.recv_async() => {
                let _ = handle_uart_msg(&session, sample, &mut known_uart_nodes, &topology, args.delay_ns, &mut rng).await;
            }
            Ok(sample) = sysc_sub.recv_async() => {
                let _ = handle_sysc_msg(&session, sample, &mut known_sysc_nodes, &topology, args.delay_ns).await;
            }
            Ok(sample) = rf_802154_sub.recv_async() => {
                let positions = node_positions.read().await;
                let _ = handle_rf_msg(&session, sample, &mut known_rf_nodes, RfCtx {
                    topic_prefix: "sim/rf/802154", positions: &positions,
                    args: &args, has_rf_header: true, obstacles: &obstacles,
                }).await;
            }
            Ok(sample) = rf_hci_sub.recv_async() => {
                let positions = node_positions.read().await;
                let _ = handle_rf_msg(&session, sample, &mut known_rf_nodes, RfCtx {
                    topic_prefix: "sim/rf/hci", positions: &positions,
                    args: &args, has_rf_header: false, obstacles: &obstacles,
                }).await;
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
            Ok(sample) = pos_sub.recv_async() => {
                let payload_bytes = sample.payload().to_bytes();
                if let Ok(payload_str) = std::str::from_utf8(&payload_bytes) {
                    if let Ok(update) = serde_json::from_str::<PositionUpdate>(payload_str) {
                        let mut positions = node_positions.write().await;
                        let entry = positions.entry(update.id.clone()).or_insert(NodeInfo {
                            id: update.id.clone(),
                            x: 0.0, y: 0.0, z: 0.0,
                        });
                        entry.x = update.x;
                        entry.y = update.y;
                        entry.z = update.z;
                        println!("Position Update: node={} x={:.2} y={:.2} z={:.2}",
                                 update.id, update.x, update.y, update.z);
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
) -> Result<(), Box<dyn std::error::Error>> {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 5 {
        return Ok(());
    }
    let sender_id = parts[3].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return Ok(());
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>()?;
    let size = cursor.read_u32::<LittleEndian>()?;

    // Validation: ensure payload actually contains 'size' bytes after header
    if payload.len() < (12 + size as usize) {
        eprintln!("ETH: Packet from {} has mismatched size (expected {}, got {})", 
                  sender_id, size, payload.len() - 12);
        return Ok(());
    }

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
            continue;
        }

        let new_delivery_vtime_ns = delivery_vtime_ns.saturating_add(delay_ns);

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns)?;
        new_payload.write_u32::<LittleEndian>(size)?;
        new_payload.write_all(&payload[12..])?;

        let rx_topic = format!("sim/eth/frame/{}/rx", node);
        let _ = session.put(&rx_topic, new_payload).await;
    }
    Ok(())
}

async fn handle_uart_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    topology: &HashMap<(String, String), LinkState>,
    default_delay_ns: u64,
    rng: &mut ChaCha8Rng,
) -> Result<(), Box<dyn std::error::Error>> {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 4 {
        return Ok(());
    }
    let sender_id = parts[2].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return Ok(());
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>()?;
    let size = cursor.read_u32::<LittleEndian>()?;

    if payload.len() < (12 + size as usize) {
        return Ok(());
    }

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
            continue;
        }

        let new_delivery_vtime_ns = delivery_vtime_ns.saturating_add(delay_ns);

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns)?;
        new_payload.write_u32::<LittleEndian>(size)?;
        new_payload.write_all(&payload[12..])?;

        let rx_topic = format!("virtmcu/uart/{}/rx", node);
        let _ = session.put(&rx_topic, new_payload).await;
    }
    Ok(())
}

async fn handle_sysc_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    topology: &HashMap<(String, String), LinkState>,
    default_delay_ns: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 5 {
        return Ok(());
    }
    let sender_id = parts[3].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return Ok(());
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>()?;
    let size = cursor.read_u32::<LittleEndian>()?;

    if payload.len() < (12 + size as usize) {
        return Ok(());
    }

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

        let new_delivery_vtime_ns = delivery_vtime_ns.saturating_add(delay_ns);

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns)?;
        new_payload.write_u32::<LittleEndian>(size)?;
        new_payload.write_all(&payload[12..])?;

        let rx_topic = format!("sim/systemc/frame/{}/rx", node);
        let _ = session.put(&rx_topic, new_payload).await;
    }
    Ok(())
}

/// Read-only RF routing context passed into `handle_rf_msg`.
struct RfCtx<'a> {
    topic_prefix: &'a str,
    positions: &'a HashMap<String, NodeInfo>,
    args: &'a Args,
    has_rf_header: bool,
    obstacles: &'a [ObstacleBox],
}

async fn handle_rf_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    ctx: RfCtx<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let RfCtx { topic_prefix, positions, args, has_rf_header, obstacles } = ctx;
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    let sender_id = parts[parts.len() - 2].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();

    // Decode header: 802.15.4 frames use FlatBuffer RfHeader; HCI frames use
    // the legacy 12-byte packed header (delivery_vtime_ns:u64 + size:u32).
    let (delivery_vtime_ns, size, orig_rssi, orig_lqi, payload_offset) = if has_rf_header {
        match rf_header::decode(&payload) {
            Some((vt, sz, rssi, lqi)) => {
                // Header size = size-prefix (4) + FlatBuffer body (le32 value at [0..4])
                let fb_len = if payload.len() >= 4 {
                    4 + u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]])
                        as usize
                } else {
                    return Ok(());
                };
                (vt, sz, rssi, lqi, fb_len)
            }
            None => return Ok(()),
        }
    } else {
        if payload.len() < 12 {
            return Ok(());
        }
        let mut cursor = Cursor::new(&payload);
        let vt = cursor.read_u64::<LittleEndian>()?;
        let sz = cursor.read_u32::<LittleEndian>()?;
        (vt, sz, 0i8, 255u8, 12)
    };
    let _ = orig_rssi; // used only in the re-encode path below via rssi_f

    if payload.len() < payload_offset + size as usize {
        return Ok(());
    }
    let frame_data = &payload[payload_offset..payload_offset + size as usize];

    let sender_pos = positions.get(&sender_id);

    // Spatial index: only check nodes in adjacent grid cells (Phase 14.6).
    let candidate_ids: Vec<String> = if let Some(spos) = sender_pos {
        let grid = SpatialGrid::build(positions);
        grid.candidates(spos.x, spos.y, spos.z, &sender_id)
    } else {
        known_nodes.iter().filter(|id| *id != &sender_id).cloned().collect()
    };

    for receiver_id in &candidate_ids {
        let receiver_pos = positions.get(receiver_id);

        let mut rssi_f = args.tx_power;
        let mut extra_delay_ns = args.delay_ns;

        if let (Some(s), Some(r)) = (sender_pos, receiver_pos) {
            let dist = ((s.x - r.x).powi(2) + (s.y - r.y).powi(2) + (s.z - r.z).powi(2)).sqrt();
            if dist.is_normal() || dist == 0.0 {
                let path_loss = calculate_fspl(dist, 2.4e9);
                if !path_loss.is_nan() {
                    rssi_f -= path_loss as f32;
                }
                // Sum attenuation from every obstacle whose AABB the TX→RX segment crosses.
                let obstacle_db: f64 = obstacles
                    .iter()
                    .filter(|obs| ray_intersects_aabb(s.x, s.y, s.z, r.x, r.y, r.z, obs))
                    .map(|obs| obs.attenuation_db)
                    .sum();
                rssi_f -= obstacle_db as f32;

                let dist_delay = (dist * 3.33) as u64;
                extra_delay_ns = extra_delay_ns.saturating_add(dist_delay);
            }
        }

        if rssi_f < args.sensitivity {
            continue;
        }

        let new_vtime = delivery_vtime_ns.saturating_add(extra_delay_ns);

        let new_payload: Vec<u8> = if has_rf_header {
            let clamped_rssi = rssi_f.clamp(-128.0, 127.0) as i8;
            let mut buf = rf_header::encode(new_vtime, size, clamped_rssi, orig_lqi);
            buf.extend_from_slice(frame_data);
            buf
        } else {
            let mut buf = Vec::with_capacity(12 + frame_data.len());
            let _ = buf.write_u64::<LittleEndian>(new_vtime);
            let _ = buf.write_u32::<LittleEndian>(size);
            buf.extend_from_slice(frame_data);
            buf
        };

        let rx_topic = format!("{}/{}/rx", topic_prefix, receiver_id);
        let _ = session.put(&rx_topic, new_payload).await;
    }
    Ok(())
}

fn calculate_fspl(dist_m: f64, freq_hz: f64) -> f64 {
    if dist_m < 0.1 { return 0.0; }
    let c = 299_792_458.0;
    // FSPL (dB) = 20 log10(d) + 20 log10(f) + 20 log10(4π/c)
    20.0 * dist_m.log10() + 20.0 * freq_hz.log10() + 20.0 * (4.0 * std::f64::consts::PI / c).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_20db() -> ObstacleBox {
        ObstacleBox {
            x_min: 4.9, x_max: 5.1,
            y_min: -100.0, y_max: 100.0,
            z_min: -100.0, z_max: 100.0,
            attenuation_db: 20.0,
        }
    }

    #[test]
    fn test_ray_passes_through_wall() {
        // Sender (0,0,0) → Receiver (10,0,0) crosses wall at x=5
        assert!(ray_intersects_aabb(0.0, 0.0, 0.0, 10.0, 0.0, 0.0, &wall_20db()));
    }

    #[test]
    fn test_ray_misses_wall_parallel() {
        // Ray parallel to the wall plane (x is constant at -1, never enters wall)
        assert!(!ray_intersects_aabb(-1.0, -10.0, 0.0, -1.0, 10.0, 0.0, &wall_20db()));
    }

    #[test]
    fn test_ray_misses_wall_same_side() {
        // Both endpoints on the same side of the wall
        assert!(!ray_intersects_aabb(0.0, 0.0, 0.0, 4.0, 0.0, 0.0, &wall_20db()));
    }

    #[test]
    fn test_ray_diagonal_through_wall() {
        // Diagonal ray from (0,0,0) to (10,3,0) — still crosses x=5 plane within y bounds
        assert!(ray_intersects_aabb(0.0, 0.0, 0.0, 10.0, 3.0, 0.0, &wall_20db()));
    }

    #[test]
    fn test_ray_diagonal_misses_wall_y() {
        // Ray goes from (0,0,0) to (10, 200, 0) — crosses x=5 at y=100, just outside y_max
        assert!(!ray_intersects_aabb(0.0, 0.0, 0.0, 10.0, 210.0, 0.0, &wall_20db()));
    }

    #[test]
    fn test_obstacle_attenuation_reduces_rssi() {
        // Open space: sender (0,0,0), receiver (10,0,0), no obstacles
        let tx_power: f32 = 0.0;
        let fspl = calculate_fspl(10.0, 2.4e9) as f32;
        let rssi_open = tx_power - fspl;

        // Same geometry but with a 20 dB wall at x=5
        let wall = wall_20db();
        let obstacle_db: f64 = [&wall]
            .iter()
            .filter(|obs| ray_intersects_aabb(0.0, 0.0, 0.0, 10.0, 0.0, 0.0, obs))
            .map(|obs| obs.attenuation_db)
            .sum();
        let rssi_wall = tx_power - fspl - obstacle_db as f32;

        let diff = rssi_open - rssi_wall;
        assert!(
            (diff - 20.0).abs() < 0.01,
            "Expected 20 dB attenuation, got {diff:.2} dB"
        );
    }

    #[test]
    fn test_multiple_obstacles_sum() {
        // Two walls in line-of-sight: each 10 dB → total 20 dB
        let wall_a = ObstacleBox { x_min: 2.9, x_max: 3.1, y_min: -100.0, y_max: 100.0, z_min: -100.0, z_max: 100.0, attenuation_db: 10.0 };
        let wall_b = ObstacleBox { x_min: 6.9, x_max: 7.1, y_min: -100.0, y_max: 100.0, z_min: -100.0, z_max: 100.0, attenuation_db: 10.0 };
        let obstacles = [wall_a, wall_b];
        let total: f64 = obstacles.iter()
            .filter(|obs| ray_intersects_aabb(0.0, 0.0, 0.0, 10.0, 0.0, 0.0, obs))
            .map(|obs| obs.attenuation_db)
            .sum();
        assert!((total - 20.0).abs() < 0.01, "Expected 20 dB total, got {total:.2} dB");
    }

    #[test]
    fn test_obstacle_not_in_path_no_attenuation() {
        // Wall is beside the path, not crossing it
        let side_wall = ObstacleBox { x_min: 4.9, x_max: 5.1, y_min: 50.0, y_max: 100.0, z_min: -100.0, z_max: 100.0, attenuation_db: 30.0 };
        assert!(!ray_intersects_aabb(0.0, 0.0, 0.0, 10.0, 0.0, 0.0, &side_wall));
    }
}

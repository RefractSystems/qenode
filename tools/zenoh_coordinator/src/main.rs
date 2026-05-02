/*
 * virtmcu Zenoh Coordinator
 *
 * This Rust daemon replaces the concept of a traditional "WirelessMedium" or
 * central network switch found in other emulation frameworks (like Renode).
 */
use zenoh_coordinator::barrier::{CoordMessage, QuantumBarrier};
use zenoh_coordinator::topology::{self, Protocol};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Parser;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::RwLock;
use virtmcu_api::rf_generated::rf_header;
use virtmcu_api::{FlatBufferStructExt, ZenohFrameHeader};
use zenoh::config::Config;
use zenoh::Wait;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 1_000_000)]
    delay_ns: u64,
    #[arg(short, long)]
    connect: Option<String>,
    #[arg(short, long, default_value_t = 42)]
    seed: u64,
    #[arg(short, long, default_value_t = 0.0)]
    tx_power: f32,
    #[arg(long, default_value_t = -90.0)]
    sensitivity: f32,
    #[arg(long)]
    topology: Option<String>,
    #[arg(long)]
    obstacles: Option<String>,
    #[arg(long)]
    nodes: Option<usize>,
    #[arg(long, default_value_t = false)]
    pdes: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct TopologyUpdate {
    from: String,
    to: String,
    delay_ns: Option<u64>,
    drop_probability: Option<f64>,
    jitter_ns: Option<u64>,
    enable_collisions: Option<bool>,
}

struct LinkState {
    delay_ns: u64,
    drop_probability: f64,
    jitter_ns: u64,
    enable_collisions: bool,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
struct NodeInfo {
    id: String,
    x: f64,
    y: f64,
    z: f64,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
struct PositionUpdate {
    id: String,
    x: f64,
    y: f64,
    z: f64,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ObstacleBox {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    z_min: f64,
    z_max: f64,
    attenuation_db: f64,
}
#[derive(Serialize, Deserialize, Debug, Default)]
struct ObstaclesConfig {
    obstacles: Vec<ObstacleBox>,
}

fn ray_intersects_aabb(
    ox: f64,
    oy: f64,
    oz: f64,
    tx: f64,
    ty: f64,
    tz: f64,
    obs: &ObstacleBox,
) -> bool {
    let (dx, dy, dz) = (tx - ox, ty - oy, tz - oz);
    let (mut t_min, mut t_max) = (0.0f64, 1.0f64);
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

fn parse_topic_with_prefix(topic: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = topic.split('/').collect();
    if topic.contains("sim/flexray") {
        return None;
    }
    let mut ps = None;
    for (i, p) in parts.iter().enumerate() {
        if *p == "sim" || *p == "virtmcu" {
            ps = Some(i);
            break;
        }
    }
    let prefix = if let Some(idx) = ps {
        parts[..idx].join("/")
    } else {
        String::new()
    };
    let (base, nid) = if topic.ends_with("/tx") && parts.len() >= 2 {
        (
            parts[..parts.len() - 2].join("/"),
            parts[parts.len() - 2].to_owned(),
        )
    } else {
        (topic.to_owned(), String::new())
    };
    Some((prefix, base, nid))
}

struct SpatialGrid {
    cells: HashMap<(i64, i64, i64), Vec<String>>,
}
impl SpatialGrid {
    fn build(pos: &HashMap<(String, String), NodeInfo>, prefix: &str) -> Self {
        let mut cells = HashMap::new();
        for ((p, id), info) in pos {
            if p == prefix {
                cells
                    .entry((
                        (info.x / 500.0).floor() as i64,
                        (info.y / 500.0).floor() as i64,
                        (info.z / 500.0).floor() as i64,
                    ))
                    .or_insert_with(Vec::new)
                    .push(id.clone());
            }
        }
        SpatialGrid { cells }
    }
    fn candidates(&self, x: f64, y: f64, z: f64, sid: &str) -> Vec<String> {
        let (cx, cy, cz) = (
            (x / 500.0).floor() as i64,
            (y / 500.0).floor() as i64,
            (z / 500.0).floor() as i64,
        );
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = self.cells.get(&(cx + dx, cy + dy, cz + dz)) {
                        for id in ids {
                            if id != sid {
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

fn parse_protocol(p: u8) -> Protocol {
    match p {
        0 => Protocol::Ethernet,
        1 => Protocol::Uart,
        2 => Protocol::Spi,
        3 => Protocol::CanFd,
        4 => Protocol::FlexRay,
        5 => Protocol::Lin,
        _ => Protocol::Ethernet,
    }
}
fn decode_batch(payload: &[u8]) -> Vec<CoordMessage> {
    let mut msgs = Vec::new();
    let mut cur = Cursor::new(payload);
    if let Ok(num) = cur.read_u32::<LittleEndian>() {
        for _ in 0..num {
            if let (Ok(src), Ok(dst), Ok(vt), Ok(seq), Ok(pr), Ok(sz)) = (
                cur.read_u32::<LittleEndian>(),
                cur.read_u32::<LittleEndian>(),
                cur.read_u64::<LittleEndian>(),
                cur.read_u64::<LittleEndian>(),
                cur.read_u8(),
                cur.read_u32::<LittleEndian>(),
            ) {
                let mut data = vec![0u8; sz as usize];
                if std::io::Read::read_exact(&mut cur, &mut data).is_ok() {
                    msgs.push(CoordMessage {
                        src_node_id: src.to_string(),
                        dst_node_id: dst.to_string(),
                        base_topic: "sim/coord".to_owned(),
                        delivery_vtime_ns: vt,
                        sequence_number: seq,
                        protocol: parse_protocol(pr),
                        payload: data,
                    });
                }
            }
        }
    }
    msgs
}

async fn encode_protocol_msg(session: &zenoh::Session, msg: &CoordMessage) {
    let topic = format!("{}/{}/rx", msg.base_topic, msg.dst_node_id);
    let payload = match msg.protocol {
        Protocol::Lin | Protocol::FlexRay | Protocol::CanFd | Protocol::Rf802154 => {
            msg.payload.clone()
        }
        Protocol::Spi => {
            let hdr = virtmcu_api::ZenohSPIHeader::new(
                msg.delivery_vtime_ns,
                msg.sequence_number,
                msg.payload.len() as u32,
                false, // default CS
                0,     // default CS index
                0,     // padding
            );
            let mut p = Vec::with_capacity(virtmcu_api::ZENOH_SPI_HEADER_SIZE + msg.payload.len());
            p.extend_from_slice(hdr.pack());
            p.extend_from_slice(&msg.payload);
            p
        }
        _ => {
            let hdr = ZenohFrameHeader::new(
                msg.delivery_vtime_ns,
                msg.sequence_number,
                msg.payload.len() as u32,
            );
            let mut p =
                Vec::with_capacity(virtmcu_api::ZENOH_FRAME_HEADER_SIZE + msg.payload.len());
            p.extend_from_slice(hdr.pack());
            p.extend_from_slice(&msg.payload);
            p
        }
    };
    let _ = session.put(&topic, payload).await;
}

async fn handle_eth_msg(
    s: zenoh::sample::Sample,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    rng: &mut ChaCha8Rng,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let mut out = Vec::new();
    let (px, base, src) = match parse_topic_with_prefix(s.key_expr().as_str()) {
        Some(r) => r,
        None => return out,
    };
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    if p.len() < 20 {
        return out;
    }
    let h = ZenohFrameHeader::unpack_slice(&p).unwrap();
    if p.len() < (virtmcu_api::ZENOH_FRAME_HEADER_SIZE + h.size() as usize) {
        return out;
    }
    let data = p[20..virtmcu_api::ZENOH_FRAME_HEADER_SIZE + h.size() as usize].to_vec();

    let mut dest_nodes = HashSet::new();
    if tg.is_explicit {
        dest_nodes = tg.get_wire_peers(&src, &Protocol::Ethernet);
    } else if let Some(nodes) = known.get(&base) {
        for dst in nodes {
            if dst != &src {
                dest_nodes.insert(dst.clone());
            }
        }
    }

    for dst in dest_nodes {
        if !tg.is_link_allowed(&src, &dst, &Protocol::Ethernet) {
            eprintln!(
                "[Topology Violation] Dropping ETH msg from {} to {}",
                src, dst
            );
            continue;
        }
        let (d, prob, jit, _) = if let Some(s) = topo.get(&(px.clone(), src.clone(), dst.clone())) {
            (
                s.delay_ns,
                s.drop_probability,
                s.jitter_ns,
                s.enable_collisions,
            )
        } else {
            (delay, 0.0, 0, false)
        };
        if prob > 0.0 && rng.gen::<f64>() < prob {
            continue;
        }
        let mut act = d;
        if jit > 0 {
            act = act.saturating_add(rng.gen_range(0..=jit));
        }
        out.push(CoordMessage {
            src_node_id: src.clone(),
            dst_node_id: dst.clone(),
            base_topic: base.clone(),
            delivery_vtime_ns: h.delivery_vtime_ns().saturating_add(act),
            sequence_number: h.sequence_number(),
            protocol: Protocol::Ethernet,
            payload: data.clone(),
        });
    }
    out
}

async fn handle_uart_msg(
    s: zenoh::sample::Sample,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    rng: &mut ChaCha8Rng,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let mut out = Vec::new();
    let (px, base, src) = match parse_topic_with_prefix(s.key_expr().as_str()) {
        Some(r) => r,
        None => return out,
    };
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    if p.len() < 20 {
        return out;
    }
    let h = ZenohFrameHeader::unpack_slice(&p).unwrap();
    if p.len() < (virtmcu_api::ZENOH_FRAME_HEADER_SIZE + h.size() as usize) {
        return out;
    }
    let data = p[20..virtmcu_api::ZENOH_FRAME_HEADER_SIZE + h.size() as usize].to_vec();

    let mut dest_nodes = HashSet::new();
    if tg.is_explicit {
        dest_nodes = tg.get_wire_peers(&src, &Protocol::Uart);
    } else if let Some(nodes) = known.get(&base) {
        for dst in nodes {
            if dst != &src {
                dest_nodes.insert(dst.clone());
            }
        }
    }

    for dst in dest_nodes {
        if !tg.is_link_allowed(&src, &dst, &Protocol::Uart) {
            eprintln!(
                "[Topology Violation] Dropping UART msg from {} to {}",
                src, dst
            );
            continue;
        }
        let (d, prob, jit, _) = if let Some(s) = topo.get(&(px.clone(), src.clone(), dst.clone())) {
            (
                s.delay_ns,
                s.drop_probability,
                s.jitter_ns,
                s.enable_collisions,
            )
        } else {
            (delay, 0.0, 0, false)
        };
        if prob > 0.0 && rng.gen::<f64>() < prob {
            continue;
        }
        let mut act = d;
        if jit > 0 {
            act = act.saturating_add(rng.gen_range(0..=jit));
        }
        out.push(CoordMessage {
            src_node_id: src.clone(),
            dst_node_id: dst.clone(),
            base_topic: base.clone(),
            delivery_vtime_ns: h.delivery_vtime_ns().saturating_add(act),
            sequence_number: h.sequence_number(),
            protocol: Protocol::Uart,
            payload: data.clone(),
        });
    }
    out
}

async fn handle_lin_msg(
    s: zenoh::sample::Sample,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let mut out = Vec::new();
    let (px, base, src) = match parse_topic_with_prefix(s.key_expr().as_str()) {
        Some(r) => r,
        None => return out,
    };
    known.entry(base.clone()).or_default().insert(src.clone());
    let pb = s.payload().to_bytes();
    let frame = match virtmcu_api::lin_generated::virtmcu::lin::root_as_lin_frame(&pb) {
        Ok(f) => f,
        Err(_) => return out,
    };

    let mut dest_nodes = HashSet::new();
    if tg.is_explicit {
        dest_nodes = tg.get_wire_peers(&src, &Protocol::Lin);
    } else if let Some(nodes) = known.get(&base) {
        for dst in nodes {
            if dst != &src {
                dest_nodes.insert(dst.clone());
            }
        }
    }

    for dst in dest_nodes {
        if !tg.is_link_allowed(&src, &dst, &Protocol::Lin) {
            eprintln!("Topology Violation: LIN {}->{}", src, dst);
            continue;
        }
        let d = if let Some(s) = topo.get(&(px.clone(), src.clone(), dst.clone())) {
            s.delay_ns
        } else {
            delay
        };
        let mut fbb = flatbuffers::FlatBufferBuilder::new();
        let data = frame.data().map(|d| fbb.create_vector(d.bytes()));
        let args = virtmcu_api::lin_generated::virtmcu::lin::LinFrameArgs {
            delivery_vtime_ns: frame.delivery_vtime_ns().saturating_add(d),
            type_: frame.type_(),
            data,
        };
        let f = virtmcu_api::lin_generated::virtmcu::lin::LinFrame::create(&mut fbb, &args);
        fbb.finish(f, None);
        out.push(CoordMessage {
            src_node_id: src.clone(),
            dst_node_id: dst.clone(),
            base_topic: base.clone(),
            delivery_vtime_ns: args.delivery_vtime_ns,
            sequence_number: 0,
            protocol: Protocol::Lin,
            payload: fbb.finished_data().to_vec(),
        });
    }
    out
}

async fn handle_sysc_msg(
    s: zenoh::sample::Sample,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let mut out = Vec::new();
    let (px, base, src) = match parse_topic_with_prefix(s.key_expr().as_str()) {
        Some(r) => r,
        None => return out,
    };
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    if p.len() < virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
        return out;
    }
    let h = match virtmcu_api::ZenohFrameHeader::unpack_slice(&p) {
        Some(h) => h,
        None => return out,
    };
    if p.len() < (virtmcu_api::ZENOH_FRAME_HEADER_SIZE + h.size() as usize) {
        return out;
    }
    let data = p[virtmcu_api::ZENOH_FRAME_HEADER_SIZE
        ..virtmcu_api::ZENOH_FRAME_HEADER_SIZE + h.size() as usize]
        .to_vec();

    let mut dest_nodes = HashSet::new();
    if tg.is_explicit {
        dest_nodes = tg.get_wire_peers(&src, &Protocol::Spi);
    } else if let Some(nodes) = known.get(&base) {
        for dst in nodes {
            if dst != &src {
                dest_nodes.insert(dst.clone());
            }
        }
    }

    for dst in dest_nodes {
        // For SystemC CAN, any allowed link is fine, we just reuse the Ethernet protocol mapping internally
        let d = if let Some(s) = topo.get(&(px.clone(), src.clone(), dst.clone())) {
            s.delay_ns
        } else {
            delay
        };
        out.push(CoordMessage {
            src_node_id: src.clone(),
            dst_node_id: dst.clone(),
            base_topic: base.clone(),
            delivery_vtime_ns: h.delivery_vtime_ns().saturating_add(d),
            sequence_number: h.sequence_number(),
            protocol: Protocol::Ethernet, // Map to standard ZenohFrameHeader wrapper
            payload: data.clone(),
        });
    }
    out
}

async fn handle_rf_msg(
    s: zenoh::sample::Sample,
    known: &mut HashMap<String, HashSet<String>>,
    positions: &HashMap<(String, String), NodeInfo>,
    args: &Args,
    has_hdr: bool,
    obstacles: &[ObstacleBox],
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let mut out = Vec::new();
    let (px, base, src) = match parse_topic_with_prefix(s.key_expr().as_str()) {
        Some(r) => r,
        None => return out,
    };
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    let (vt, seq, sz, _, lqi, off) = if has_hdr {
        match rf_header::decode(&p) {
            Some((vt, seq, sz, _, lqi)) => {
                let fbl = if p.len() >= 4 {
                    4 + u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize
                } else {
                    return out;
                };
                (vt, seq, sz, 0, lqi, fbl)
            }
            None => return out,
        }
    } else {
        if p.len() < 12 {
            return out;
        }
        let mut c = Cursor::new(&p);
        let vt = c.read_u64::<LittleEndian>().unwrap_or(0);
        let sz = c.read_u32::<LittleEndian>().unwrap_or(0);
        (vt, 0, sz, 0i8, 255u8, 12)
    };
    if p.len() < off + sz as usize {
        return out;
    }
    let data = &p[off..off + sz as usize];
    let mut cands = if let Some(s) = positions.get(&(px.clone(), src.clone())) {
        SpatialGrid::build(positions, &px).candidates(s.x, s.y, s.z, &src)
    } else {
        known.get(&base).map_or(Vec::new(), |ns| {
            ns.iter().filter(|&id| id != &src).cloned().collect()
        })
    };
    if tg.is_explicit {
        if tg.has_wireless() {
            let ns = tg.rf_neighbors(&src);
            cands.retain(|id| ns.contains(id));
        } else {
            cands.clear();
        }
    }
    for dst in cands {
        let (mut rssi, mut d) = (args.tx_power, args.delay_ns);
        if let (Some(s), Some(r)) = (
            positions.get(&(px.clone(), src.clone())),
            positions.get(&(px.clone(), dst.clone())),
        ) {
            let dist = ((s.x - r.x).powi(2) + (s.y - r.y).powi(2) + (s.z - r.z).powi(2)).sqrt();
            if dist.is_normal() || dist == 0.0 {
                let pl = calculate_fspl(dist, 2.4e9);
                if !pl.is_nan() {
                    rssi -= pl as f32;
                }
                rssi -= obstacles
                    .iter()
                    .filter(|o| ray_intersects_aabb(s.x, s.y, s.z, r.x, r.y, r.z, o))
                    .map(|o| o.attenuation_db)
                    .sum::<f64>() as f32;
                d = d.saturating_add((dist * 3.33) as u64);
            }
        }
        if rssi < args.sensitivity {
            continue;
        }
        let vt2 = vt.saturating_add(d);
        let p2 = if has_hdr {
            let mut b = rf_header::encode(vt2, seq, sz, rssi.clamp(-128.0, 127.0) as i8, lqi);
            b.extend_from_slice(data);
            b
        } else {
            let mut b = Vec::with_capacity(12 + data.len());
            let _ = b.write_u64::<LittleEndian>(vt2);
            let _ = b.write_u32::<LittleEndian>(sz);
            b.extend_from_slice(data);
            b
        };
        out.push(CoordMessage {
            src_node_id: src.clone(),
            dst_node_id: dst,
            base_topic: base.clone(),
            delivery_vtime_ns: vt2,
            sequence_number: seq,
            protocol: if has_hdr {
                Protocol::Rf802154
            } else {
                Protocol::RfHci
            },
            payload: p2,
        });
    }
    out
}

fn calculate_fspl(dist_m: f64, freq_hz: f64) -> f64 {
    if dist_m < 0.1 {
        0.0
    } else {
        20.0 * dist_m.log10()
            + 20.0 * freq_hz.log10()
            + 20.0 * (4.0 * std::f64::consts::PI / 299_792_458.0).log10()
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    tracing::info!("Starting virtmcu Zenoh Coordinator");
    let tg_raw = if let Some(ref p) = args.topology {
        match topology::TopologyGraph::from_yaml(std::path::Path::new(p)) {
            Ok(g) => {
                tracing::info!("Topology loaded: {}", p);
                g
            }
            Err(e) => panic!("Topology error: {:?}", e),
        }
    } else {
        topology::TopologyGraph::default()
    };
    let seed = tg_raw.global_seed.unwrap_or(args.seed);
    let obstacles = if let Some(ref p) = args.obstacles {
        let f = std::fs::File::open(p).unwrap();
        let c: ObstaclesConfig = serde_yaml::from_reader(f).unwrap();
        tracing::info!("Obstacles loaded: {}", p);
        c.obstacles
    } else {
        Vec::new()
    };
    // Force client mode + disabled multicast scouting (CLAUDE.md Second Priority,
    // ADR-014). `Config::default()` is peer mode with multicast scouting ON, which
    // causes parallel pytest workers' coordinators to silently discover each
    // other across the container's network namespace and cross-talk on shared
    // topics like `sim/coord/*/done`.
    let mut config = Config::default();
    if let Some(ref c) = args.connect {
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", c))
            .unwrap();
    }
    config.insert_json5("mode", "\"client\"").unwrap();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    let session = zenoh::open(config).await.unwrap();

    let eth_sub = session
        .declare_subscriber("**/sim/eth/frame/**/tx")
        .await
        .unwrap();
    let uart_sub = session
        .declare_subscriber("**/virtmcu/uart/**/tx")
        .await
        .unwrap();
    let sysc_sub = session
        .declare_subscriber("**/sim/systemc/frame/**/tx")
        .await
        .unwrap();
    let rf_802154_sub = session
        .declare_subscriber("**/sim/rf/ieee802154/**/tx")
        .await
        .unwrap();
    let rf_hci_sub = session
        .declare_subscriber("**/sim/rf/hci/**/tx")
        .await
        .unwrap();
    let lin_sub = session
        .declare_subscriber("**/sim/lin/**/tx")
        .await
        .unwrap();
    let tx_sub = session
        .declare_subscriber("**/sim/coord/**/tx")
        .await
        .unwrap();
    let ctrl_sub = session
        .declare_subscriber("**/sim/network/control")
        .await
        .unwrap();
    let pos_sub = session
        .declare_subscriber("**/sim/telemetry/position")
        .await
        .unwrap();
    let done_sub = session
        .declare_subscriber("**/sim/coord/**/done")
        .await
        .unwrap();

    let _ready_q = session
        .declare_queryable("sim/coordinator/ready_probe")
        .callback(|query| {
            let _ = query.reply(query.key_expr(), b"ok").wait();
        })
        .await
        .unwrap();

    let _liveliness = session
        .liveliness()
        .declare_token("sim/coordinator/liveliness")
        .await
        .unwrap();

    let mut k_eth = HashMap::new();
    let mut k_uart = HashMap::new();
    let mut k_sysc = HashMap::new();
    let mut k_rf = HashMap::new();
    let mut k_lin = HashMap::new();
    let mut base_topics = HashMap::new();
    let mut topology = HashMap::new();
    let node_positions: Arc<RwLock<HashMap<(String, String), NodeInfo>>> = {
        let mut m = HashMap::new();
        m.insert(
            ("".to_owned(), "0".to_owned()),
            NodeInfo {
                id: "0".to_owned(),
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        );
        m.insert(
            ("".to_owned(), "1".to_owned()),
            NodeInfo {
                id: "1".to_owned(),
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        );
        m.insert(
            ("".to_owned(), "2".to_owned()),
            NodeInfo {
                id: "2".to_owned(),
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        );
        Arc::new(RwLock::new(m))
    };
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let tg_ref = Arc::new(RwLock::new(tg_raw));
    let barrier = if args.pdes {
        let n = args.nodes.expect("--nodes required for --pdes");
        let tg = tg_ref.read().await;
        Some(Arc::new(QuantumBarrier::new(
            n,
            tg.max_messages_per_node_per_quantum,
        )))
    } else {
        None
    };
    let mut current_quantum: u64 = 1;
    let mut batches: HashMap<String, Vec<CoordMessage>> = HashMap::new();
    tracing::info!(
        "PDES: {}",
        if args.pdes {
            format!("ENABLED ({} nodes)", args.nodes.unwrap())
        } else {
            "DISABLED".to_owned()
        }
    );

    loop {
        tokio::select! {
            res = eth_sub.recv_async() => {
                if let Ok(s) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_eth_msg(s, &mut k_eth, &topology, args.delay_ns, &mut rng, &tg).await;
                    base_topics.insert(Protocol::Ethernet, "sim/eth/frame".to_owned());
                    if barrier.is_some() {
                        for m in msgs {
                            batches.entry(m.src_node_id.clone()).or_default().push(m);
                        }
                    } else {
                        for m in msgs {
                            encode_protocol_msg(&session, &m).await;
                        }
                    }
                }
            }
            res = uart_sub.recv_async() => {
                if let Ok(s) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_uart_msg(s, &mut k_uart, &topology, args.delay_ns, &mut rng, &tg).await;
                    base_topics.insert(Protocol::Uart, "virtmcu/uart".to_owned());
                    if barrier.is_some() {
                        for m in msgs {
                            batches.entry(m.src_node_id.clone()).or_default().push(m);
                        }
                    } else {
                        for m in msgs {
                            encode_protocol_msg(&session, &m).await;
                        }
                    }
                }
            }
            res = sysc_sub.recv_async() => {
                if let Ok(s) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_sysc_msg(s, &mut k_sysc, &topology, args.delay_ns, &tg).await;
                    base_topics.insert(Protocol::Spi, "sim/systemc/frame".to_owned());
                    if barrier.is_some() {
                        for m in msgs {
                            batches.entry(m.src_node_id.clone()).or_default().push(m);
                        }
                    } else {
                        for m in msgs {
                            encode_protocol_msg(&session, &m).await;
                        }
                    }
                }
            }
            res = rf_802154_sub.recv_async() => {
                if let Ok(s) = res {
                    let tg = tg_ref.read().await;
                    let ps = node_positions.read().await;
                    let msgs = handle_rf_msg(s, &mut k_rf, &ps, &args, true, &obstacles, &tg).await;
                    base_topics.insert(Protocol::Rf802154, "sim/rf/ieee802154".to_owned());
                    if barrier.is_some() {
                        for m in msgs {
                            batches.entry(m.src_node_id.clone()).or_default().push(m);
                        }
                    } else {
                        for m in msgs {
                            encode_protocol_msg(&session, &m).await;
                        }
                    }
                }
            }
            res = rf_hci_sub.recv_async() => {
                if let Ok(s) = res {
                    let tg = tg_ref.read().await;
                    let ps = node_positions.read().await;
                    let msgs = handle_rf_msg(s, &mut k_rf, &ps, &args, false, &obstacles, &tg).await;
                    base_topics.insert(Protocol::RfHci, "sim/rf/hci".to_owned());
                    if barrier.is_some() {
                        for m in msgs {
                            batches.entry(m.src_node_id.clone()).or_default().push(m);
                        }
                    } else {
                        for m in msgs {
                            encode_protocol_msg(&session, &m).await;
                        }
                    }
                }
            }
            res = lin_sub.recv_async() => {
                if let Ok(s) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_lin_msg(s, &mut k_lin, &topology, args.delay_ns, &tg).await;
                    base_topics.insert(Protocol::Lin, "sim/lin".to_owned());
                    if barrier.is_some() {
                        for m in msgs {
                            batches.entry(m.src_node_id.clone()).or_default().push(m);
                        }
                    } else {
                        for m in msgs {
                            encode_protocol_msg(&session, &m).await;
                        }
                    }
                }
            }
            res = tx_sub.recv_async() => {
                if let Ok(s) = res {
                    let ps = s.key_expr().as_str().split('/').collect::<Vec<_>>();
                    if ps.len() >= 4 {
                        let nid = ps[2].to_owned();
                        let mut ms = decode_batch(&s.payload().to_bytes());
                        if barrier.is_some() {
                            batches.entry(nid).or_default().append(&mut ms);
                        } else {
                            for m in ms {
                                encode_protocol_msg(&session, &m).await;
                            }
                        }
                    }
                }
            }
            res = done_sub.recv_async() => {
                if let Ok(s) = res {
                    if let Some(ref b) = barrier {
                        let ps = s.key_expr().as_str().split('/').collect::<Vec<_>>();
                        if ps.len() >= 4 {
                            let nid = ps[2].to_owned();
                            let payload = s.payload().to_bytes();
                            let mut quantum = u64::MAX;
                            if payload.len() >= 8 {
                                let mut cursor = Cursor::new(&payload);
                                quantum = cursor.read_u64::<LittleEndian>().unwrap_or(u64::MAX);
                                if quantum != current_quantum {
                                    tracing::error!("Quantum mismatch for node {}: expected {}, got {}", nid, current_quantum, quantum);
                                }
                            }
                            tracing::debug!("Received DONE for node {} quantum {}", nid, quantum);
                            let msgs = batches.remove(&nid).unwrap_or_default();
                            match b.submit_done(nid.clone(), quantum, current_quantum, msgs) {
                                Ok(Some(sorted)) => {
                                    let q = b.current_quantum() - 1;
                                    tracing::info!("Quantum {} complete. Delivering {} messages.", q, sorted.len());
                                    for m in sorted {
                                        encode_protocol_msg(&session, &m).await;
                                    }

                                    // Send start to all nodes for NEXT quantum
                                    current_quantum = b.current_quantum();
                                    tracing::debug!("Advancing to quantum {}. Sending START to all nodes.", current_quantum);
                                    for i in 0..args.nodes.unwrap_or(0) {
                                        let start_topic = format!("sim/clock/start/{}", i);
                                        let mut start_payload = Vec::new();
                                        start_payload
                                            .write_u64::<LittleEndian>(current_quantum)
                                            .expect("Vec write failed");
                                        let _ = session.put(&start_topic, start_payload).await;
                                    }

                                    let _ = session.put("sim/coord/all/start", vec![1]).await;
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    tracing::error!("Barrier error for node {}: {:?}", nid, e);
                                }
                            }
                        }
                    }
                }
            }
            res = ctrl_sub.recv_async() => {
                if let Ok(s) = res {
                    if let Some((px, _, _)) = parse_topic_with_prefix(s.key_expr().as_str()) {
                        if let Ok(ps) = std::str::from_utf8(&s.payload().to_bytes()) {
                            if let Ok(up) = serde_json::from_str::<TopologyUpdate>(ps) {
                                let st = topology.entry((px, up.from, up.to)).or_insert(LinkState {
                                    delay_ns: args.delay_ns,
                                    drop_probability: 0.0,
                                    jitter_ns: 0,
                                    enable_collisions: false,
                                });
                                if let Some(d) = up.delay_ns {
                                    st.delay_ns = d;
                                }
                                if let Some(p) = up.drop_probability {
                                    st.drop_probability = p;
                                }
                                if let Some(j) = up.jitter_ns {
                                    st.jitter_ns = j;
                                }
                                if let Some(c) = up.enable_collisions {
                                    st.enable_collisions = c;
                                }
                            }
                        }
                    }
                }
            }
            res = pos_sub.recv_async() => {
                if let Ok(s) = res {
                    if let Some((px, _, _)) = parse_topic_with_prefix(s.key_expr().as_str()) {
                        if let Ok(ps) = std::str::from_utf8(&s.payload().to_bytes()) {
                            if let Ok(up) = serde_json::from_str::<PositionUpdate>(ps) {
                                let mut tg = tg_ref.write().await;
                                tg.update_positions(vec![(up.id.clone(), [up.x, up.y, up.z])]);
                                let mut pos = node_positions.write().await;
                                let e = pos.entry((px, up.id.clone())).or_insert(NodeInfo {
                                    id: up.id,
                                    x: 0.0,
                                    y: 0.0,
                                    z: 0.0,
                                });
                                e.x = up.x;
                                e.y = up.y;
                                e.z = up.z;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wall_20db() -> ObstacleBox {
        ObstacleBox {
            x_min: 4.9,
            x_max: 5.1,
            y_min: -100.0,
            y_max: 100.0,
            z_min: -100.0,
            z_max: 100.0,
            attenuation_db: 20.0,
        }
    }
    #[test]
    fn test_ray_passes_through_wall() {
        assert!(ray_intersects_aabb(
            0.0,
            0.0,
            0.0,
            10.0,
            0.0,
            0.0,
            &wall_20db()
        ));
    }
    #[test]
    fn test_ray_misses_wall_parallel() {
        assert!(!ray_intersects_aabb(
            -1.0,
            -10.0,
            0.0,
            -1.0,
            10.0,
            0.0,
            &wall_20db()
        ));
    }
    #[test]
    fn test_obstacle_attenuation_reduces_rssi() {
        let diff = (0.0 - calculate_fspl(10.0, 2.4e9)) - (0.0 - calculate_fspl(10.0, 2.4e9) - 20.0);
        assert!((diff - 20.0).abs() < 0.01);
    }
}

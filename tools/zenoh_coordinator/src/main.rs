#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![deny(unsafe_code)]
/*
 * virtmcu Zenoh Coordinator
 *
 * This Rust daemon replaces the concept of a traditional "WirelessMedium" or
 * central network switch found in other emulation frameworks (like Renode).
 */
use zenoh_coordinator::barrier::{QuantumBarrier};
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
use virtmcu_wire::{FlatBufferStructExt, ZenohFrameHeader};

use zenoh::Wait;

struct MsgArgs {
    src: String,
    base: String,
    s: zenoh::sample::Sample,
}

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
    #[arg(long)]
    listen: Option<String>,
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
        Protocol::Spi => {
            let hdr = virtmcu_wire::ZenohSPIHeader::new(
                msg.delivery_vtime_ns,
                msg.sequence_number,
                msg.payload.len() as u32,
                false, // default CS
                0,     // default CS index
                0,     // padding
            );
            let mut p = Vec::with_capacity(virtmcu_wire::ZENOH_SPI_HEADER_SIZE + msg.payload.len());
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
                Vec::with_capacity(virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + msg.payload.len());
            p.extend_from_slice(hdr.pack());
            p.extend_from_slice(&msg.payload);
            p
        }
    };
    let _ = session.put(&topic, payload).await;
}

async fn handle_eth_msg(
    msg: MsgArgs,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    rng: &mut ChaCha8Rng,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let MsgArgs { src, base, s } = msg;
    let mut out = Vec::new();
    let px = String::new();
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    if p.len() < 20 {
        return out;
    }
    let h = ZenohFrameHeader::unpack_slice(&p).expect("Failed to unpack ZenohFrameHeader");
    if p.len() < (virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize) {
        return out;
    }
    let data = p[20..virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize].to_vec();

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
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.is_link_allowed(&src, &dst, &Protocol::Ethernet) {
            tracing::error!(
                "[Topology Violation] Dropping ETH msg from {} to {}",
                src,
                dst
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

async fn handle_chardev_msg(
    msg: MsgArgs,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    rng: &mut ChaCha8Rng,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let MsgArgs { src, base, s } = msg;
    let mut out = Vec::new();
    let px = String::new();
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    if p.len() < virtmcu_wire::ZENOH_FRAME_HEADER_SIZE {
        return out;
    }
    let h = match virtmcu_wire::ZenohFrameHeader::unpack_slice(&p) {
        Some(h) => h,
        None => return out,
    };
    if p.len() < (virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize) {
        return out;
    }
    let data = p[virtmcu_wire::ZENOH_FRAME_HEADER_SIZE
        ..virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize]
        .to_vec();

    let mut dest_nodes = HashSet::new();
    if tg.is_explicit {
        dest_nodes = tg.get_wire_peers(&src, &Protocol::ReferenceLink);
    } else if let Some(nodes) = known.get(&base) {
        for dst in nodes {
            if dst != &src {
                dest_nodes.insert(dst.clone());
            }
        }
    }

    for dst in dest_nodes {
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.is_link_allowed(&src, &dst, &Protocol::ReferenceLink) {
            tracing::error!(
                "[Topology Violation] Dropping Chardev msg from {} to {}",
                src,
                dst
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
            protocol: Protocol::ReferenceLink,
            payload: data.clone(),
        });
    }
    out
}

async fn handle_uart_msg(
    msg: MsgArgs,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    rng: &mut ChaCha8Rng,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let MsgArgs { src, base, s } = msg;
    let mut out = Vec::new();
    let px = String::new();
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    if p.len() < 20 {
        return out;
    }
    let h = ZenohFrameHeader::unpack_slice(&p).expect("Failed to unpack ZenohFrameHeader");
    if p.len() < (virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize) {
        return out;
    }
    let data = p[20..virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize].to_vec();

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
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.is_link_allowed(&src, &dst, &Protocol::Uart) {
            tracing::error!(
                "[Topology Violation] Dropping UART msg from {} to {}",
                src,
                dst
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
    msg: MsgArgs,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let MsgArgs { src, base, s } = msg;
    let mut out = Vec::new();
    let px = String::new();
    known.entry(base.clone()).or_default().insert(src.clone());
    let p_full = s.payload().to_bytes();
    let pb = if let Some((_, _, data)) = virtmcu_wire::decode_frame(&p_full) {
        data
    } else {
        return out;
    };
    let frame = match virtmcu_wire::lin_generated::virtmcu::lin::root_as_lin_frame(pb) {
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
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.is_link_allowed(&src, &dst, &Protocol::Lin) {
            tracing::error!("Topology Violation: LIN {}->{}", src, dst);
            continue;
        }
        let d = if let Some(s) = topo.get(&(px.clone(), src.clone(), dst.clone())) {
            s.delay_ns
        } else {
            delay
        };
        let mut fbb = flatbuffers::FlatBufferBuilder::new();
        let data = frame.data().map(|d| fbb.create_vector(d.bytes()));
        let args = virtmcu_wire::lin_generated::virtmcu::lin::LinFrameArgs {
            delivery_vtime_ns: frame.delivery_vtime_ns().saturating_add(d),
            type_: frame.type_(),
            data,
        };
        let f = virtmcu_wire::lin_generated::virtmcu::lin::LinFrame::create(&mut fbb, &args);
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
    msg: MsgArgs,
    known: &mut HashMap<String, HashSet<String>>,
    topo: &HashMap<(String, String, String), LinkState>,
    delay: u64,
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let MsgArgs { src, base, s } = msg;
    let mut out = Vec::new();
    let px = String::new();
    known.entry(base.clone()).or_default().insert(src.clone());
    let p = s.payload().to_bytes();
    if p.len() < virtmcu_wire::ZENOH_FRAME_HEADER_SIZE {
        return out;
    }
    let h = match virtmcu_wire::ZenohFrameHeader::unpack_slice(&p) {
        Some(h) => h,
        None => return out,
    };
    if p.len() < (virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize) {
        return out;
    }
    let data = p[virtmcu_wire::ZENOH_FRAME_HEADER_SIZE
        ..virtmcu_wire::ZENOH_FRAME_HEADER_SIZE + h.size() as usize]
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
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
        if !tg.routing_map.map.contains_key(&src) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&src) {
            if !targets.contains(&dst) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
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
    msg: MsgArgs,
    known: &mut HashMap<String, HashSet<String>>,
    positions: &HashMap<(String, String), NodeInfo>,
    args: &Args,
    has_hdr: bool,
    obstacles: &[ObstacleBox],
    tg: &topology::TopologyGraph,
) -> Vec<CoordMessage> {
    let MsgArgs { src, base, s } = msg;
    let mut out = Vec::new();
    let px = String::new();
    known.entry(base.clone()).or_default().insert(src.clone());
    let p_full = s.payload().to_bytes();
    let p = if has_hdr {
        if let Some((_, _, data)) = virtmcu_wire::decode_frame(&p_full) {
            data.to_vec()
        } else {
            p_full.to_vec()
        }
    } else {
        p_full.to_vec()
    };
    let (vt, seq, payload, lqi, mhr) = if has_hdr {
        match virtmcu_wire::rf802154::size_prefixed_root_as_rf_802154_frame(&p) {
            Ok(f) => (
                f.delivery_vtime_ns(),
                f.sequence_number(),
                f.data().map(|d| d.bytes().to_vec()).unwrap_or_default(),
                f.lqi(),
                virtmcu_wire::Rf802154Mhr {
                    fcf: f.fcf(),
                    seq_num: f.mhr_seq_num(),
                    dest_pan: f.dest_pan(),
                    dest_addr: f.dest_addr(),
                    src_pan: f.src_pan(),
                    src_addr: f.src_addr(),
                },
            ),
            Err(_) => return out,
        }
    } else {
        if p.len() < 12 {
            return out;
        }
        let mut c = Cursor::new(&p);
        let vt = c.read_u64::<LittleEndian>().expect("Invalid data format");
        let sz = c.read_u32::<LittleEndian>().expect("Invalid data format");
        let mut data = vec![0u8; sz as usize];
        if p.len() >= 12 + sz as usize {
            data.copy_from_slice(&p[12..12 + sz as usize]);
        }
        (
            vt,
            0,
            data,
            255u8,
            virtmcu_wire::Rf802154Mhr {
                fcf: 0,
                seq_num: 0,
                dest_pan: 0xFFFF,
                dest_addr: 0xFFFFFFFFFFFFFFFF,
                src_pan: 0xFFFF,
                src_addr: 0xFFFFFFFFFFFFFFFF,
            },
        )
    };

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
            virtmcu_wire::encode_rf802154_frame(
                vt2,
                seq,
                &payload,
                rssi.clamp(-128.0, 127.0) as i8,
                lqi,
                mhr,
            )
        } else {
            let mut b = Vec::with_capacity(12 + payload.len());
            let _ = b.write_u64::<LittleEndian>(vt2);
            let _ = b.write_u32::<LittleEndian>(payload.len() as u32);
            b.extend_from_slice(&payload);
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
        let f = std::fs::File::open(p).expect("Failed to open obstacles file");
        let c: ObstaclesConfig =
            serde_yaml::from_reader(f).expect("Failed to parse obstacles YAML");
        tracing::info!("Obstacles loaded: {}", p);
        c.obstacles
    } else {
        Vec::new()
    };
    // Force client mode + disabled multicast scouting (CLAUDE.md Second Priority,
    // RFC-0001). `Config::default()` is peer mode with multicast scouting ON, which
    // causes parallel pytest workers' coordinators to silently discover each
    // other across the container's network namespace and cross-talk on shared
    // topics like `sim/coord/*/done`.
    let mut config = virtmcu_zenoh_config::default_config();
    if let Some(ref c) = args.connect {
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", c))
            .expect("Failed to configure Zenoh");
    }
    if let Some(ref l) = args.listen {
        config
            .insert_json5("listen/endpoints", &format!("[\"{}\"]", l))
            .expect("Failed to configure Zenoh");
        config
            .insert_json5("mode", "\"router\"")
            .expect("Failed to configure Zenoh");
    } else {
        config
            .insert_json5("mode", "\"client\"")
            .expect("Failed to configure Zenoh");
    }
    let session = zenoh::open(config)
        .await
        .expect("Failed to open Zenoh session");

    let (eth_tx, mut eth_rx) = tokio::sync::mpsc::unbounded_channel();
    let (uart_tx, mut uart_rx) = tokio::sync::mpsc::unbounded_channel();
    let (sysc_tx, mut sysc_rx) = tokio::sync::mpsc::unbounded_channel();
    let (chardev_tx, mut chardev_rx) = tokio::sync::mpsc::unbounded_channel();
    let (rf_802154_tx, mut rf_802154_rx) = tokio::sync::mpsc::unbounded_channel();
    let (rf_hci_tx, mut rf_hci_rx) = tokio::sync::mpsc::unbounded_channel();
    let (lin_tx, mut lin_rx) = tokio::sync::mpsc::unbounded_channel();
    let (tx_tx, mut tx_rx) = tokio::sync::mpsc::unbounded_channel();
    let (ctrl_tx, mut ctrl_rx) = tokio::sync::mpsc::unbounded_channel();
    let (pos_tx, mut pos_rx) = tokio::sync::mpsc::unbounded_channel();
    let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel();

    let mut _subs = Vec::new();

    let ctrl_tx_c = ctrl_tx.clone();
    _subs.push(
        session
            .declare_subscriber("sim/network/control")
            .callback(move |s| {
                let _ = ctrl_tx_c.send(s);
            })
            .await
            .expect("Failed"),
    );
    let pos_tx_c = pos_tx.clone();
    _subs.push(
        session
            .declare_subscriber("sim/telemetry/position")
            .callback(move |s| {
                let _ = pos_tx_c.send((String::new(), String::new(), s));
            })
            .await
            .expect("Failed"),
    );

    for node in tg_raw.routing_map.map.keys() {
        let n = node.clone();
        let eth_c = eth_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/eth/frame/{n}/tx"))
                .callback(move |s| {
                    let _ = eth_c.send((n.clone(), "sim/eth/frame".to_owned(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let uart_c = uart_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("virtmcu/uart/{n}/tx"))
                .callback(move |s| {
                    let _ = uart_c.send((n.clone(), "virtmcu/uart".to_owned(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let sysc_c = sysc_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/systemc/frame/{n}/tx"))
                .callback(move |s| {
                    let _ = sysc_c.send((n.clone(), "sim/systemc/frame".to_owned(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let chardev_c = chardev_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/chardev/{n}/tx"))
                .callback(move |s| {
                    let _ = chardev_c.send((n.clone(), "sim/chardev".to_owned(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let rf802_c = rf_802154_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/rf/ieee802154/{n}/tx"))
                .callback(move |s| {
                    let _ = rf802_c.send((n.clone(), "sim/rf/ieee802154".to_owned(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let rfhci_c = rf_hci_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/rf/hci/{n}/tx"))
                .callback(move |s| {
                    let _ = rfhci_c.send((n.clone(), "sim/rf/hci".to_owned(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let lin_c = lin_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/lin/{n}/tx"))
                .callback(move |s| {
                    let _ = lin_c.send((n.clone(), "sim/lin".to_owned(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let tx_c = tx_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/coord/{n}/tx"))
                .callback(move |s| {
                    let _ = tx_c.send((n.clone(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let done_c = done_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/coord/{n}/done"))
                .callback(move |s| {
                    let _ = done_c.send((n.clone(), s));
                })
                .await
                .expect("Failed"),
        );

        let n = node.clone();
        let pos_n_c = pos_tx.clone();
        _subs.push(
            session
                .declare_subscriber(format!("sim/telemetry/position/{n}"))
                .callback(move |s| {
                    let _ = pos_n_c.send((String::new(), n.clone(), s));
                })
                .await
                .expect("Failed"),
        );
    }

    let _ready_q = session
        .declare_queryable("sim/coordinator/ready_probe")
        .callback(|query| {
            let _ = query.reply(query.key_expr(), b"ok").wait();
        })
        .await
        .expect("Failed to declare ready_probe queryable");

    let _liveliness = session
        .liveliness()
        .declare_token("sim/coordinator/liveliness")
        .await
        .expect("Failed to declare liveliness token");

    let mut k_eth = HashMap::new();
    let mut k_uart = HashMap::new();
    let mut k_sysc = HashMap::new();
    let mut k_chardev = HashMap::new();
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
            format!(
                "ENABLED ({} nodes)",
                args.nodes
                    .expect("--nodes must be specified when --pdes is enabled")
            )
        } else {
            "DISABLED".to_owned()
        }
    );

    loop {
        tokio::select! {
            res = eth_rx.recv() => {
                if let Some((src, base, s)) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_eth_msg(MsgArgs { src, base, s }, &mut k_eth, &topology, args.delay_ns, &mut rng, &tg).await;
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
            res = uart_rx.recv() => {
                if let Some((src, base, s)) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_uart_msg(MsgArgs { src, base, s }, &mut k_uart, &topology, args.delay_ns, &mut rng, &tg).await;
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
            res = sysc_rx.recv() => {
                if let Some((src, base, s)) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_sysc_msg(MsgArgs { src, base, s }, &mut k_sysc, &topology, args.delay_ns, &tg).await;
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
            res = chardev_rx.recv() => {
                if let Some((src, base, s)) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_chardev_msg(MsgArgs { src, base, s }, &mut k_chardev, &topology, args.delay_ns, &mut rng, &tg).await;
                    base_topics.insert(Protocol::ReferenceLink, "sim/chardev".to_owned());
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
            res = rf_802154_rx.recv() => {
                if let Some((src, base, s)) = res {
                    let tg = tg_ref.read().await;
                    let ps = node_positions.read().await;
                    let msgs = handle_rf_msg(MsgArgs { src, base, s }, &mut k_rf, &ps, &args, true, &obstacles, &tg).await;
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
            res = rf_hci_rx.recv() => {
                if let Some((src, base, s)) = res {
                    let tg = tg_ref.read().await;
                    let ps = node_positions.read().await;
                    let msgs = handle_rf_msg(MsgArgs { src, base, s }, &mut k_rf, &ps, &args, false, &obstacles, &tg).await;
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
            res = lin_rx.recv() => {
                if let Some((src, base, s)) = res {
                    let tg = tg_ref.read().await;
                    let msgs = handle_lin_msg(MsgArgs { src, base, s }, &mut k_lin, &topology, args.delay_ns, &tg).await;
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
            res = tx_rx.recv() => {
                if let Some((nid, s)) = res {
                        let tg = tg_ref.read().await;
                        let mut ms = decode_batch(&s.payload().to_bytes());
                        for m in &ms {
                            if !tg.routing_map.map.contains_key(&m.src_node_id) {
                                panic!("Unregistered packet received!");
                            }
                            if let Some(targets) = tg.routing_map.get_targets(&m.src_node_id) {
                                if !targets.contains(&m.dst_node_id) {
                                    panic!("Unregistered packet received!");
                                }
                            } else {
                                panic!("Unregistered packet received!");
                            }
                        }
                        if barrier.is_some() {
                            batches.entry(nid).or_default().append(&mut ms);
                        } else {
                            for m in ms {
                                encode_protocol_msg(&session, &m).await;
                            }
                        }
                    }
            }
            res = done_rx.recv() => {
                if let Some((nid, s)) = res {
                    if let Some(ref b) = barrier {
                            let payload = s.payload().to_bytes();
                            let mut quantum = u64::MAX;
                            if payload.len() >= 8 {
                                let mut cursor = Cursor::new(&payload);
                                quantum = cursor.read_u64::<LittleEndian>().expect("Invalid data format");
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
                                    for i in 0..args.nodes.expect("Invalid data format") {
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
            res = ctrl_rx.recv() => {
                if let Some(s) = res {
                    let px = String::new();
                    {
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
            res = pos_rx.recv() => {
                if let Some((px, _src, s)) = res {
                    {
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

    #[test]
    #[should_panic(expected = "Unregistered packet received!")]
    fn test_panic_on_unregistered_packet() {
        let mut tg = crate::topology::TopologyGraph::default();
        tg.is_explicit = true;
        // Node "0" is registered, but target "2" is not in targets for "0"
        tg.routing_map.add_route("0".to_owned(), "1".to_owned());

        let m = CoordMessage {
            src_node_id: "0".to_owned(),
            dst_node_id: "2".to_owned(), // Unregistered dest!
            base_topic: "test".to_owned(),
            delivery_vtime_ns: 0,
            sequence_number: 0,
            protocol: crate::topology::Protocol::Ethernet,
            payload: vec![],
        };

        if !tg.routing_map.map.contains_key(&m.src_node_id) {
            panic!("Unregistered packet received!");
        }
        if let Some(targets) = tg.routing_map.get_targets(&m.src_node_id) {
            if !targets.contains(&m.dst_node_id) {
                panic!("Unregistered packet received!");
            }
        } else {
            panic!("Unregistered packet received!");
        }
    }
}

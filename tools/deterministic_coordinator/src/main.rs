use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Parser;
use std::io::Cursor;
use std::sync::Arc;

use deterministic_coordinator::barrier::{CoordMessage, QuantumBarrier};
use deterministic_coordinator::topology::{self, Protocol};

#[derive(Parser, Debug)]
#[command(version, about = "Deterministic Coordinator", long_about = None)]
struct Args {
    #[arg(long, default_value_t = 3)]
    nodes: usize,

    #[arg(short, long)]
    connect: Option<String>,

    #[arg(long)]
    topology: Option<String>,
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

fn serialize_protocol(p: &Protocol) -> u8 {
    match p {
        Protocol::Ethernet => 0,
        Protocol::Uart => 1,
        Protocol::Spi => 2,
        Protocol::CanFd => 3,
        Protocol::FlexRay => 4,
        Protocol::Lin => 5,
    }
}

fn decode_batch(payload: &[u8]) -> Vec<CoordMessage> {
    let mut msgs = Vec::new();
    let mut cursor = Cursor::new(payload);
    if let Ok(num_msgs) = cursor.read_u32::<LittleEndian>() {
        for _ in 0..num_msgs {
            if let (Ok(src), Ok(dst), Ok(vtime), Ok(seq), Ok(proto), Ok(len)) = (
                cursor.read_u32::<LittleEndian>(),
                cursor.read_u32::<LittleEndian>(),
                cursor.read_u64::<LittleEndian>(),
                cursor.read_u64::<LittleEndian>(),
                cursor.read_u8(),
                cursor.read_u32::<LittleEndian>(),
            ) {
                let mut data = vec![0u8; len as usize];
                if std::io::Read::read_exact(&mut cursor, &mut data).is_ok() {
                    msgs.push(CoordMessage {
                        src_node_id: src,
                        dst_node_id: dst,
                        delivery_vtime_ns: vtime,
                        sequence_number: seq,
                        protocol: parse_protocol(proto),
                        payload: data,
                    });
                }
            }
        }
    }
    msgs
}

fn encode_message(msg: &CoordMessage) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.write_u32::<LittleEndian>(msg.src_node_id).unwrap();
    buf.write_u32::<LittleEndian>(msg.dst_node_id).unwrap();
    buf.write_u64::<LittleEndian>(msg.delivery_vtime_ns)
        .unwrap();
    buf.write_u64::<LittleEndian>(msg.sequence_number).unwrap();
    buf.write_u8(serialize_protocol(&msg.protocol)).unwrap();
    buf.write_u32::<LittleEndian>(msg.payload.len() as u32)
        .unwrap();
    buf.extend_from_slice(&msg.payload);
    buf
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("DeterministicCoordinator starting...");

    let args = Args::parse();

    let topo = if let Some(ref path) = args.topology {
        topology::TopologyGraph::from_yaml(std::path::Path::new(path)).unwrap_or_else(|e| {
            tracing::error!("Failed to load topology: {}", e);
            topology::TopologyGraph::default()
        })
    } else {
        topology::TopologyGraph::default()
    };

    let max_messages = topo.max_messages_per_node_per_quantum;
    let barrier = Arc::new(QuantumBarrier::new(args.nodes, max_messages));

    let mut config = zenoh::Config::default();
    if let Some(router) = args.connect {
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", router))
            .unwrap();
    }

    let session = zenoh::open(config).await.unwrap();

    let done_sub = session
        .declare_subscriber("sim/coord/*/done")
        .await
        .unwrap();
    let tx_sub = session.declare_subscriber("sim/coord/*/tx").await.unwrap();

    let mut node_batches = std::collections::HashMap::new();

    loop {
        tokio::select! {
            Ok(sample) = tx_sub.recv_async() => {
                let topic = sample.key_expr().as_str();
                let parts: Vec<&str> = topic.split('/').collect();
                if parts.len() >= 4 {
                    if let Ok(node_id) = parts[2].parse::<u32>() {
                        let mut msgs = decode_batch(&sample.payload().to_bytes());
                        node_batches.entry(node_id).or_insert_with(Vec::new).append(&mut msgs);
                    }
                }
            }
            Ok(sample) = done_sub.recv_async() => {
                let topic = sample.key_expr().as_str();
                let parts: Vec<&str> = topic.split('/').collect();
                if parts.len() >= 4 {
                    if let Ok(node_id) = parts[2].parse::<u32>() {
                        let msgs = node_batches.remove(&node_id).unwrap_or_default();

                        match barrier.submit_done(node_id, msgs) {
                            Ok(Some(sorted_msgs)) => {
                                // All nodes done, deliver messages
                                for msg in sorted_msgs {
                                    let rx_topic = format!("sim/coord/{}/rx", msg.dst_node_id);
                                    let payload = encode_message(&msg);
                                    let _ = session.put(&rx_topic, payload).await;
                                }

                                // Send start to all nodes
                                for i in 0..args.nodes {
                                    let start_topic = format!("sim/coord/{}/start", i);
                                    let _ = session.put(&start_topic, vec![1]).await;
                                }

                                barrier.reset();
                            }
                            Ok(None) => {
                                // Waiting for others
                            }
                            Err(e) => {
                                tracing::error!("Barrier error for node {}: {:?}", node_id, e);
                            }
                        }
                    }
                }
            }
        }
    }
}

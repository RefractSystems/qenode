use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Parser;
use deterministic_coordinator::message_log::MessageLog;
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
    #[arg(long)]
    pcap_log: Option<String>,
}

fn parse_protocol(p: u8) -> Protocol {
    match p {
        0 => Protocol::Ethernet,
        1 => Protocol::Uart,
        2 => Protocol::Spi,
        3 => Protocol::CanFd,
        4 => Protocol::FlexRay,
        5 => Protocol::Lin,
        6 => Protocol::Rf802154,
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
        Protocol::Rf802154 => 6,
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
    buf.write_u32::<LittleEndian>(msg.src_node_id)
        .expect("Vec write failed");
    buf.write_u32::<LittleEndian>(msg.dst_node_id)
        .expect("Vec write failed");
    buf.write_u64::<LittleEndian>(msg.delivery_vtime_ns)
        .expect("Vec write failed");
    buf.write_u64::<LittleEndian>(msg.sequence_number)
        .expect("Vec write failed");
    buf.write_u8(serialize_protocol(&msg.protocol))
        .expect("Vec write failed");
    buf.write_u32::<LittleEndian>(msg.payload.len() as u32)
        .expect("Vec write failed");
    buf.extend_from_slice(&msg.payload);
    buf
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    tracing::info!("DeterministicCoordinator starting...");

    let args = Args::parse();

    let topo = if let Some(path) = &args.topology {
        match topology::TopologyGraph::from_yaml(std::path::Path::new(path)) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("Failed to load topology: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        topology::TopologyGraph::default()
    };

    let pcap_log = if let Some(path) = &args.pcap_log {
        match MessageLog::create(std::path::Path::new(path)) {
            Ok(log) => Some(log),
            Err(e) => {
                tracing::error!("Failed to create PCAP log at {}: {}", path, e);
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    let max_messages = topo.max_messages_per_node_per_quantum;
    let barrier = Arc::new(QuantumBarrier::new(args.nodes, max_messages));

    if topo.transport == topology::Transport::Unix {
        barrier.set_quantum(1);
        run_unix_coordinator(args, topo, barrier, pcap_log).await
    } else {
        barrier.set_quantum(1);
        run_zenoh_coordinator(args, topo, barrier, pcap_log).await
    }
}

async fn run_zenoh_coordinator(
    args: Args,
    topo: topology::TopologyGraph,
    barrier: Arc<QuantumBarrier>,
    mut pcap_log: Option<MessageLog>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = zenoh::Config::default();
    config
        .insert_json5("mode", "\"client\"")
        .map_err(|e| format!("Invalid Zenoh mode: {}", e))?;

    if let Some(router) = args.connect {
        tracing::info!("Connecting to Zenoh router at {}", router);
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", router))
            .map_err(|e| format!("Invalid Zenoh endpoint: {}", e))?;
    }

    let session = zenoh::open(config)
        .await
        .map_err(|e| format!("Failed to open Zenoh session: {}", e))?;

    // ARCH-8: Declare liveliness so nodes know a coordinator is active
    let liveliness_topic = "sim/coord/alive";
    let _liveliness = session
        .liveliness()
        .declare_token(liveliness_topic)
        .await
        .map_err(|e| format!("Failed to declare liveliness token: {}", e))?;
    tracing::info!(
        "Coordinator liveliness token declared on {}",
        liveliness_topic
    );

    let sub = session
        .declare_subscriber("sim/coord/*/*")
        .await
        .map_err(|e| format!("Failed to declare subscriber: {}", e))?;
    tracing::info!("Coordinator subscriber active");

    let mut node_batches = std::collections::HashMap::new();
    let mut current_quantum: u64 = 1;

    while let Ok(sample) = sub.recv_async().await {
        let topic = sample.key_expr().as_str();
        let parts: Vec<&str> = topic.split('/').collect();
        if parts.len() >= 4 {
            if let Ok(node_id) = parts[2].parse::<u32>() {
                let action = parts[3];
                if action == "tx" {
                    let mut msgs = decode_batch(&sample.payload().to_bytes());
                    node_batches
                        .entry(node_id)
                        .or_insert_with(Vec::new)
                        .append(&mut msgs);
                } else if action == "done" {
                    let payload = sample.payload().to_bytes();
                    let mut quantum = u64::MAX;
                    if payload.len() >= 8 {
                        let mut cursor = Cursor::new(&payload);
                        quantum = cursor.read_u64::<LittleEndian>().unwrap_or(u64::MAX);
                        if quantum != current_quantum {
                            tracing::error!(
                                "Quantum mismatch for node {}: expected {}, got {}",
                                node_id,
                                current_quantum,
                                quantum
                            );
                        }
                    }

                    let msgs = node_batches.remove(&node_id).unwrap_or_default();

                    match barrier.submit_done(node_id, quantum, current_quantum, msgs) {
                        Ok(Some(sorted_msgs)) => {
                            tracing::info!(
                                "Quantum {} complete. Delivering {} messages.",
                                current_quantum,
                                sorted_msgs.len()
                            );
                            // All nodes done, deliver messages
                            for msg in sorted_msgs {
                                let mut target_nodes = Vec::new();
                                if msg.dst_node_id == u32::MAX {
                                    // Broadcast
                                    if msg.protocol.is_wireless() {
                                        target_nodes = topo.rf_neighbors(msg.src_node_id);
                                    } else {
                                        // Wired broadcast - deliver to all nodes on the same link
                                        // This is a bit simplified: find any link that contains src and this protocol
                                        for link in topo.wire_links() {
                                            if link.protocol == msg.protocol
                                                && link.nodes.contains(&msg.src_node_id)
                                            {
                                                for &node in &link.nodes {
                                                    if node != msg.src_node_id {
                                                        target_nodes.push(node);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    if topo.is_link_allowed(
                                        msg.src_node_id,
                                        msg.dst_node_id,
                                        msg.protocol.clone(),
                                    ) {
                                        target_nodes.push(msg.dst_node_id);
                                    }
                                }

                                if target_nodes.is_empty() && msg.dst_node_id != u32::MAX {
                                    tracing::warn!(
                                        "Topology violation: dropped {} message from {} to {}",
                                        format!("{:?}", msg.protocol).to_uppercase(),
                                        msg.src_node_id,
                                        msg.dst_node_id
                                    );
                                    if let Some(log) = &mut pcap_log {
                                        if let Err(e) = log.write_topology_violation(
                                            msg.src_node_id,
                                            msg.dst_node_id,
                                            msg.delivery_vtime_ns,
                                            &msg.protocol,
                                            &msg.payload,
                                        ) {
                                            tracing::error!("Failed to write to PCAP log: {}", e);
                                        }
                                    }
                                    continue;
                                }

                                for target_node in target_nodes {
                                    if let Some(log) = &mut pcap_log {
                                        let mut logged_msg = msg.clone();
                                        logged_msg.dst_node_id = target_node;
                                        if let Err(e) = log.write_message(&logged_msg) {
                                            tracing::error!("Failed to write to PCAP log: {}", e);
                                        }
                                    }
                                    let rx_topic = format!("sim/coord/{}/rx", target_node);
                                    let mut out_msg = msg.clone();
                                    out_msg.dst_node_id = target_node; // Ensure dst matches even for broadcast
                                    let payload = encode_message(&out_msg);
                                    if let Err(e) = session.put(&rx_topic, payload).await {
                                        tracing::error!(
                                            "Failed to deliver message to {}: {}",
                                            target_node,
                                            e
                                        );
                                    }
                                }
                            }

                            if let Some(log) = &mut pcap_log {
                                if let Err(e) = log.flush() {
                                    tracing::error!("Failed to flush PCAP log: {}", e);
                                }
                            }

                            // Send start to all nodes for NEXT quantum
                            current_quantum += 1;
                            for i in 0..args.nodes {
                                let start_topic = format!("sim/clock/start/{}", i);
                                let mut start_payload = Vec::new();
                                start_payload
                                    .write_u64::<LittleEndian>(current_quantum)
                                    .expect("Vec write failed");
                                if let Err(e) = session.put(&start_topic, start_payload).await {
                                    tracing::error!("Failed to send start signal to {}: {}", i, e);
                                }
                            }
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
    Ok(())
}

async fn run_unix_coordinator(
    args: Args,
    topo: topology::TopologyGraph,
    barrier: Arc<QuantumBarrier>,
    mut pcap_log: Option<MessageLog>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    tracing::info!("Running Unix coordinator...");

    let (tx_chan, mut rx_chan) = tokio::sync::mpsc::channel::<(u32, String, Vec<u8>)>(1024);
    let node_streams = Arc::new(StdMutex::new(std::collections::HashMap::<
        u32,
        Vec<tokio::sync::mpsc::Sender<(String, Vec<u8>)>>,
    >::new()));

    for i in 0..args.nodes {
        let path = format!("/tmp/virtmcu-coord-{}.sock", i);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let tx_chan = tx_chan.clone();
        let node_streams = Arc::clone(&node_streams);
        let node_id = i as u32;

        tokio::spawn(async move {
            loop {
                if let Ok((stream, _)) = listener.accept().await {
                    // Register stream for outgoing messages
                    let (out_tx, mut out_rx) =
                        tokio::sync::mpsc::channel::<(String, Vec<u8>)>(1024);
                    {
                        node_streams
                            .lock()
                            .unwrap()
                            .entry(node_id)
                            .or_default()
                            .push(out_tx);
                    }

                    let (mut reader, mut writer) = stream.into_split();

                    // TX task for this node connection
                    tokio::spawn(async move {
                        while let Some((topic, payload)) = out_rx.recv().await {
                            let topic_bytes = topic.as_bytes();
                            let mut buf = Vec::new();
                            WriteBytesExt::write_u32::<LittleEndian>(
                                &mut buf,
                                topic_bytes.len() as u32,
                            )
                            .unwrap();
                            buf.extend_from_slice(topic_bytes);
                            WriteBytesExt::write_u32::<LittleEndian>(
                                &mut buf,
                                payload.len() as u32,
                            )
                            .unwrap();
                            buf.extend_from_slice(&payload);
                            if writer.write_all(&buf).await.is_err() {
                                break;
                            }
                        }
                    });

                    // RX loop for this node connection
                    let tx_chan = tx_chan.clone();
                    tokio::spawn(async move {
                        loop {
                            let mut topic_len_buf = [0u8; 4];
                            if reader.read_exact(&mut topic_len_buf).await.is_err() {
                                break;
                            }
                            let topic_len = u32::from_le_bytes(topic_len_buf) as usize;

                            let mut topic_buf = vec![0u8; topic_len];
                            if reader.read_exact(&mut topic_buf).await.is_err() {
                                break;
                            }
                            let topic = String::from_utf8_lossy(&topic_buf).into_owned();

                            let mut payload_len_buf = [0u8; 4];
                            if reader.read_exact(&mut payload_len_buf).await.is_err() {
                                break;
                            }
                            let payload_len = u32::from_le_bytes(payload_len_buf) as usize;

                            let mut payload = vec![0u8; payload_len];
                            if reader.read_exact(&mut payload).await.is_err() {
                                break;
                            }

                            if tx_chan.send((node_id, topic, payload)).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            }
        });
    }

    let mut node_batches = std::collections::HashMap::new();
    let mut current_quantum: u64 = 1;

    loop {
        if let Some((node_id, topic, payload)) = rx_chan.recv().await {
            if topic.ends_with("/tx") {
                let mut msgs = decode_batch(&payload);
                node_batches
                    .entry(node_id)
                    .or_insert_with(Vec::new)
                    .append(&mut msgs);
            } else if topic.ends_with("/done") {
                let mut quantum = u64::MAX;
                if payload.len() >= 8 {
                    let mut cursor = Cursor::new(&payload);
                    quantum =
                        ReadBytesExt::read_u64::<LittleEndian>(&mut cursor).unwrap_or(u64::MAX);
                }

                let msgs = node_batches.remove(&node_id).unwrap_or_default();
                match barrier.submit_done(node_id, quantum, current_quantum, msgs) {
                    Ok(Some(sorted_msgs)) => {
                        tracing::info!(
                            "Quantum {} complete (Unix). Delivering {} messages.",
                            current_quantum,
                            sorted_msgs.len()
                        );
                        for msg in sorted_msgs {
                            let mut target_nodes = Vec::new();
                            if msg.dst_node_id == u32::MAX {
                                // Broadcast
                                if msg.protocol.is_wireless() {
                                    target_nodes = topo.rf_neighbors(msg.src_node_id);
                                } else {
                                    for link in topo.wire_links() {
                                        if link.protocol == msg.protocol
                                            && link.nodes.contains(&msg.src_node_id)
                                        {
                                            for &node in &link.nodes {
                                                if node != msg.src_node_id {
                                                    target_nodes.push(node);
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                if topo.is_link_allowed(
                                    msg.src_node_id,
                                    msg.dst_node_id,
                                    msg.protocol.clone(),
                                ) {
                                    target_nodes.push(msg.dst_node_id);
                                }
                            }

                            for target_node in target_nodes {
                                if let Some(log) = &mut pcap_log {
                                    let mut logged_msg = msg.clone();
                                    logged_msg.dst_node_id = target_node;
                                    let _ = log.write_message(&logged_msg);
                                }
                                let rx_topic = format!("sim/coord/{}/rx", target_node);
                                let mut out_msg = msg.clone();
                                out_msg.dst_node_id = target_node;
                                let payload = encode_message(&out_msg);

                                let mut streams = node_streams.lock().unwrap();
                                if let Some(out_txs) = streams.get_mut(&target_node) {
                                    out_txs.retain(|tx| {
                                        tx.try_send((rx_topic.clone(), payload.clone())).is_ok()
                                    });
                                }
                            }
                        }

                        current_quantum += 1;
                        // Send start to all nodes
                        let mut streams = node_streams.lock().unwrap();
                        for (id, out_txs) in streams.iter_mut() {
                            let start_topic = format!("sim/clock/start/{}", id);
                            let mut start_payload = Vec::new();
                            WriteBytesExt::write_u64::<LittleEndian>(
                                &mut start_payload,
                                current_quantum,
                            )
                            .unwrap();
                            out_txs.retain(|tx| {
                                tx.try_send((start_topic.clone(), start_payload.clone()))
                                    .is_ok()
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("Barrier error for node {}: {:?}", node_id, e);
                    }
                }
            }
        }
    }
}

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Parser;
use deterministic_coordinator::message_log::MessageLog;
use std::io::Cursor;
use std::sync::Arc;

use deterministic_coordinator::barrier::{CoordMessage, QuantumBarrier};
use deterministic_coordinator::topology::{self, Protocol};
use virtmcu_api::{FlatBufferStructExt, ZenohFrameHeader};

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
    #[arg(long, default_value_t = false)]
    no_pdes: bool,
    #[arg(long, default_value_t = 1_000_000)]
    delay_ns: u64,
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
        7 => Protocol::RfHci,
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
        Protocol::RfHci => 7,
        Protocol::Control => 8,
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
                        base_topic: None,
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

fn parse_legacy_topic(topic: &str) -> Option<(Protocol, u32, String)> {
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() < 3 {
        return None;
    }

    if topic.contains("eth") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::Ethernet, nid, base));
            }
        }
    } else if topic.contains("uart") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::Uart, nid, base));
            }
        }
    } else if topic.contains("can") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::CanFd, nid, base));
            }
        }
    } else if topic.contains("lin") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::Lin, nid, base));
            }
        }
    } else if topic.contains("spi") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::Spi, nid, base));
            }
        }
    } else if topic.contains("rf/hci") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::RfHci, nid, base));
            }
        }
    } else if topic.contains("rf") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::Rf802154, nid, base));
            }
        }
    } else if topic.contains("systemc") {
        if let Some(nid_str) = parts.iter().rev().nth(1) {
            if let Ok(nid) = nid_str.parse::<u32>() {
                let base = parts[..parts.len() - 2].join("/");
                return Some((Protocol::Ethernet, nid, base));
            }
        }
    }

    None
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    tracing::info!("DeterministicCoordinator starting...");

    let args = Args::parse();

    let topo_raw = if let Some(path) = &args.topology {
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

    let max_messages = topo_raw.max_messages_per_node_per_quantum;
    let barrier = Arc::new(QuantumBarrier::new(args.nodes, max_messages));
    let transport = topo_raw.transport.clone();
    let topo = Arc::new(tokio::sync::RwLock::new(topo_raw));

    if transport == topology::Transport::Unix {
        barrier.set_quantum(1);
        run_unix_coordinator(args, topo, barrier, pcap_log).await
    } else {
        barrier.set_quantum(1);
        run_deterministic_coordinator(args, topo, barrier, pcap_log).await
    }
}

async fn run_deterministic_coordinator(
    args: Args,
    topo: Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    barrier: Arc<QuantumBarrier>,
    mut pcap_log: Option<MessageLog>,
) -> Result<(), Box<dyn std::error::Error>> {
    let no_pdes = args.no_pdes;
    let mut config = zenoh::Config::default();
    config
        .insert_json5("mode", "\"client\"")
        .map_err(|e| format!("Invalid Zenoh mode: {}", e))?;

    if let Some(ref router) = args.connect {
        tracing::info!("Connecting to Zenoh router at {}", router);
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", router))
            .map_err(|e| format!("Invalid Zenoh endpoint: {}", e))?;
    }
    let session = zenoh::open(config)
        .await
        .map_err(|e| format!("Failed to open Zenoh session: {}", e))?;

    let legacy_tx_topics = vec![
        deterministic_coordinator::topics::wildcard::ETH_FRAME_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::SIM_UART_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::VIRTM_UART_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::VIRTM_UART_PORT_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::CAN_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::LIN_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::SPI_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::RF_IEEE802154_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::RF_HCI_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::CHARDEV_TX_WILDCARD,
        deterministic_coordinator::topics::wildcard::NETDEV_TX_WILDCARD,
    ];

    let (tx_chan, mut rx_chan) = tokio::sync::mpsc::unbounded_channel();
    let mut _subs = Vec::new();
    for topic in &legacy_tx_topics {
        let tx = tx_chan.clone();
        let sub = session
            .declare_subscriber(*topic)
            .callback(move |sample| {
                let _ = tx.send(sample);
            })
            .await
            .map_err(|e| format!("Failed to declare subscriber for {}: {}", topic, e))?;
        _subs.push(sub);
    }
    let sub_done = session
        .declare_subscriber(deterministic_coordinator::topics::wildcard::COORD_DONE_WILDCARD)
        .await
        .map_err(|e| format!("Failed to declare done subscriber: {}", e))?;

    let sub_ctrl = session
        .declare_subscriber(deterministic_coordinator::topics::singleton::NETWORK_CONTROL)
        .await
        .map_err(|e| format!("Failed to declare control subscriber: {}", e))?;

    tracing::info!("Coordinator subscribers active");

    // Phase 4: Self-roundtrip routing barrier.
    // Declare a probe token, wait for discovery, then undeclare.
    // This ensures all previous declarations (subscribers) have been processed by the router.
    let probe_topic = format!("sim/coord/probe/{}", std::process::id());
    let probe_token = session
        .liveliness()
        .declare_token(&probe_topic)
        .await
        .map_err(|e| format!("Failed to declare probe token: {}", e))?;

    tracing::info!("Waiting for routing barrier (probe: {})...", probe_topic);
    let mut discovered = false;
    for _ in 0..50 {
        let replies = session
            .liveliness()
            .get(&probe_topic)
            .await
            .map_err(|e| format!("Liveliness get failed: {}", e))?;

        let mut count = 0;
        while let Ok(_reply) = replies.recv_async().await {
            count += 1;
        }
        if count > 0 {
            discovered = true;
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    if !discovered {
        tracing::error!("Routing barrier timeout!");
        return Err("Routing barrier timeout".into());
    }
    drop(probe_token);
    tracing::info!("Routing barrier complete.");

    let liveliness_topic = deterministic_coordinator::topics::singleton::COORD_ALIVE;
    let _liveliness = session
        .liveliness()
        .declare_token(liveliness_topic)
        .await
        .map_err(|e| format!("Failed to declare liveliness token: {}", e))?;

    let mut node_batches = std::collections::HashMap::new();
    let mut seen_nodes = std::collections::HashSet::new();
    let mut current_quantum: u64 = 1;

    for i in 0..args.nodes {
        seen_nodes.insert(i as u32);
    }

    loop {
        tokio::select! {
            Some(sample) = rx_chan.recv() => {
                let topic = sample.key_expr().as_str();
                if let Some((proto, node_id, base)) = parse_legacy_topic(topic) {
                    seen_nodes.insert(node_id);
                    tracing::info!("Received legacy {:?} TX from node {} (base: {})", proto, node_id, base);
                    let payload = sample.payload().to_bytes();

                    let mut data_opt = None;
                    let mut vtime = 0;
                    let mut seq = 0;

                    // 1. Try RfHeader if protocol is 802.15.4
                    if proto == Protocol::Rf802154 && payload.len() >= 4 {
                        let sz = u32::from_le_bytes(payload[0..4].try_into().unwrap_or([0;4])) as usize;
                        if sz > 0 && sz <= 1024 && payload.len() >= 4 + sz {
                             let hdr_slice = &payload[4..4 + sz];
                             // SAFETY: root_unchecked bypasses strict alignment check.
                             let hdr = unsafe { virtmcu_api::rf_header::RfHeader::unpack_slice_unchecked(hdr_slice) };
                             vtime = hdr.delivery_vtime_ns();
                             seq = hdr.sequence_number();
                             data_opt = Some(payload[4 + sz..].to_vec());
                        }
                    }

                    // 2. Fallback to ZenohFrameHeader if not already parsed
                    // (But only for protocols that use it!)
                    if data_opt.is_none() && proto != Protocol::Lin && proto != Protocol::CanFd && proto != Protocol::FlexRay {
                        if let Some(header) = ZenohFrameHeader::unpack_slice(&payload) {
                            let data_start = virtmcu_api::ZENOH_FRAME_HEADER_SIZE;
                            if payload.len() >= data_start + header.size() as usize {
                                vtime = header.delivery_vtime_ns();
                                seq = header.sequence_number();
                                data_opt = Some(payload[data_start..data_start + header.size() as usize].to_vec());
                            } else {
                                // MALFORMED: skip it!
                                tracing::warn!("Skipping malformed legacy frame: expected {} bytes, got {}", data_start + header.size() as usize, payload.len());
                                continue;
                            }
                        }
                    }

                    // 3. Last fallback: Raw payload (only for protocols we know are raw)
                    if data_opt.is_none() && (proto == Protocol::Lin || proto == Protocol::CanFd || proto == Protocol::FlexRay) {
                        if proto == Protocol::Lin && payload.len() >= 8 {
                             vtime = u64::from_le_bytes(payload[0..8].try_into().unwrap_or([0;8]));
                        }
                        data_opt = Some(payload.to_vec());
                    }

                    if let Some(data) = data_opt {
                        let mut msg = CoordMessage {
                            src_node_id: node_id,
                            dst_node_id: u32::MAX, // Broadcast by default for legacy
                            delivery_vtime_ns: vtime.saturating_add(args.delay_ns),
                            sequence_number: seq,
                            protocol: proto,
                            payload: data,
                            base_topic: Some(base),
                        };

                        if no_pdes {
                            deliver_message(&session, &topo, &seen_nodes, &mut pcap_log, &mut msg).await;
                        } else {
                            node_batches
                                .entry(node_id)
                                .or_insert_with(Vec::new)
                                .push(msg);
                        }
                    }
                }
            }
            Ok(sample) = sub_ctrl.recv_async() => {
                let payload = sample.payload().to_bytes();
                if let Ok(json_str) = String::from_utf8(payload.to_vec()) {
                    let mut t = topo.write().await;
                    if let Err(e) = t.update_from_json(&json_str) {
                        tracing::error!("Failed to update topology from JSON: {}", e);
                    } else {
                        tracing::info!("Topology updated from JSON: {}", json_str);
                    }
                }
            }
            Ok(sample) = sub_done.recv_async() => {
                let topic = sample.key_expr().as_str();
                let parts: Vec<&str> = topic.split('/').collect();
                if parts.len() >= 4 {
                    if let Ok(node_id) = parts[2].parse::<u32>() {
                        seen_nodes.insert(node_id);
                        let payload = sample.payload().to_bytes();
                        let mut quantum = u64::MAX;
                        let mut vtime_limit = u64::MAX;
                        let mut batched_msgs = Vec::new();
                        if payload.len() >= 8 {
                            let mut cursor = Cursor::new(&payload);
                            quantum = cursor.read_u64::<LittleEndian>().unwrap_or(u64::MAX);

                            // Check for vtime_limit in the 16-byte DONE format
                            if payload.len() >= 16 {
                                vtime_limit = cursor.read_u64::<LittleEndian>().unwrap_or(u64::MAX);
                            }

                            tracing::info!("Received DONE from node {} for quantum {} (vtime_limit: {})", node_id, quantum, vtime_limit);
                            if payload.len() > 16 {
                                batched_msgs = decode_batch(&payload[16..]);
                            } else if payload.len() > 8 && payload.len() < 16 {
                                // Legacy path for messages batched after 8-byte quantum
                                batched_msgs = decode_batch(&payload[8..]);
                            }
                        }

                        let msgs = node_batches.remove(&node_id).unwrap_or_default();
                        let (mut current_msgs, future_msgs): (Vec<CoordMessage>, Vec<CoordMessage>) = msgs.into_iter().partition(|m| m.delivery_vtime_ns <= vtime_limit);

                        if !future_msgs.is_empty() {
                            node_batches.insert(node_id, future_msgs);
                        }

                        current_msgs.append(&mut batched_msgs);

                        match barrier.submit_done(node_id, quantum, current_quantum, current_msgs) {
                            Ok(Some(mut sorted_msgs)) => {
                                sorted_msgs.sort();

                                tracing::info!(
                                    "Quantum {} complete. Delivering {} messages.",
                                    current_quantum,
                                    sorted_msgs.len()
                                );

                                for mut msg in sorted_msgs {
                                    deliver_message(&session, &topo, &seen_nodes, &mut pcap_log, &mut msg).await;
                                }

                                if let Some(log) = &mut pcap_log {
                                    let _ = log.flush();
                                }

                                current_quantum += 1;
                                for i in 0..args.nodes {
                                    let start_topic =
                                        deterministic_coordinator::topics::templates::clock_start(
                                            &i.to_string(),
                                        );
                                    let mut start_payload = Vec::new();
                                    start_payload
                                        .write_u64::<LittleEndian>(current_quantum)
                                        .expect("Vec write failed");
                                    let _ = session.put(&start_topic, start_payload).await;
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
    }
}

async fn deliver_message(
    session: &zenoh::Session,
    topo: &Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    seen_nodes: &std::collections::HashSet<u32>,
    pcap_log: &mut Option<MessageLog>,
    msg: &mut CoordMessage,
) {
    let t = topo.read().await;
    let mut target_nodes = Vec::new();
    if msg.dst_node_id == u32::MAX {
        if !t.is_explicit {
            for &nid in seen_nodes {
                if nid != msg.src_node_id
                    && t.is_link_allowed(msg.src_node_id, nid, msg.protocol.clone())
                {
                    target_nodes.push(nid);
                }
            }
        } else if msg.protocol.is_wireless() {
            target_nodes = t.rf_neighbors(msg.src_node_id);
        } else {
            for link in t.wire_links() {
                if link.protocol == msg.protocol && link.nodes.contains(&msg.src_node_id) {
                    for &node in &link.nodes {
                        if node != msg.src_node_id {
                            target_nodes.push(node);
                        }
                    }
                }
            }
        }
    } else {
        if t.is_link_allowed(msg.src_node_id, msg.dst_node_id, msg.protocol.clone()) {
            target_nodes.push(msg.dst_node_id);
        }
    }

    if target_nodes.is_empty() && msg.dst_node_id != u32::MAX {
        return;
    }

    for target_node in target_nodes {
        tracing::debug!(
            "Delivering {:?} message to node {}",
            msg.protocol,
            target_node
        );
        if let Some(log) = pcap_log {
            let mut logged_msg = msg.clone();
            logged_msg.dst_node_id = target_node;
            let _ = log.write_message(&logged_msg);
        }

        let rx_topic =
            deterministic_coordinator::topics::templates::coord_rx(&target_node.to_string());
        let mut out_msg = msg.clone();
        out_msg.dst_node_id = target_node;
        let out_payload = encode_message(&out_msg);
        let _ = session.put(&rx_topic, out_payload).await;

        let legacy_rx_topic = if let Some(base) = &msg.base_topic {
            format!("{}/{}/rx", base, target_node)
        } else {
            match msg.protocol {
                Protocol::Ethernet => {
                    deterministic_coordinator::topics::templates::eth_rx(&target_node.to_string())
                }
                Protocol::Uart => {
                    deterministic_coordinator::topics::templates::uart_rx(&target_node.to_string())
                }
                Protocol::CanFd => {
                    deterministic_coordinator::topics::templates::can_rx(&target_node.to_string())
                }
                Protocol::Lin => {
                    deterministic_coordinator::topics::templates::lin_rx(&target_node.to_string())
                }
                Protocol::Spi => {
                    // Note: SPI legacy delivery usually needs a bus name, but here we use a default
                    deterministic_coordinator::topics::templates::spi_base(
                        "default",
                        &target_node.to_string(),
                    ) + "/rx"
                }
                Protocol::Rf802154 => {
                    deterministic_coordinator::topics::templates::rf_ieee802154_rx(
                        &target_node.to_string(),
                    )
                }
                Protocol::RfHci => deterministic_coordinator::topics::templates::rf_hci_rx(
                    &target_node.to_string(),
                ),
                _ => format!("sim/unknown/{}/rx", target_node),
            }
        };
        tracing::debug!("Legacy delivery to topic: {}", legacy_rx_topic);

        let legacy_payload = match msg.protocol {
            Protocol::Rf802154 => virtmcu_api::encode_rf_frame(
                msg.delivery_vtime_ns,
                msg.sequence_number,
                &msg.payload,
                -80,
                255,
            ),
            Protocol::Lin | Protocol::CanFd | Protocol::FlexRay => msg.payload.clone(), // Raw FlatBuffer delivery
            _ => {
                virtmcu_api::encode_frame(msg.delivery_vtime_ns, msg.sequence_number, &msg.payload)
            }
        };
        let _ = session.put(&legacy_rx_topic, legacy_payload).await;
    }
}

async fn run_unix_coordinator(
    _args: Args,
    _topo: Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    _barrier: Arc<QuantumBarrier>,
    mut _pcap_log: Option<MessageLog>,
) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Unix coordinator started (minimal passthrough)");
    Ok(())
}

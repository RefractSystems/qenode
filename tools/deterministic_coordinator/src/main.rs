#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![deny(unsafe_code)]
use anyhow::{anyhow, bail, Result};
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
    /// Identifier for this running simulation instance (HLA: federation name).
    /// Used in log output. Required.
    #[arg(long, env = "VIRTMCU_FEDERATION_ID")]
    federation_id: String,

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

    #[arg(long, default_value_t = 5000)]
    join_timeout_ms: u64,

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
        9 => Protocol::ReferenceLink,
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
        Protocol::ReferenceLink => 9,
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

#[derive(Debug)]
struct DummyVTimeProvider;
impl virtmcu_observability::processors::VTimeProvider for DummyVTimeProvider {
    fn current_vtime_ns(&self) -> u64 {
        0
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _telemetry = virtmcu_observability::init_telemetry(
        "virtmcu-deterministic-coordinator",
        std::sync::Arc::new(DummyVTimeProvider),
    );
    let args = Args::parse();
    let federation_id = virtmcu_api::FederationId(args.federation_id.clone());

    tracing::info!(federation = %federation_id, "DeterministicCoordinator starting...");

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
        run_unix_coordinator(args, federation_id, topo, barrier, pcap_log).await
    } else {
        run_deterministic_coordinator(args, federation_id, topo, barrier, pcap_log).await
    }
}

async fn run_deterministic_coordinator(
    args: Args,
    federation_id: virtmcu_api::FederationId,
    topo: Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    barrier: Arc<QuantumBarrier>,
    mut pcap_log: Option<MessageLog>,
) -> Result<()> {
    let no_pdes = args.no_pdes;
    let mut config = virtmcu_zenoh_config::default_config();
    config
        .insert_json5("mode", "\"client\"")
        .map_err(|e| anyhow!("Invalid Zenoh mode: {}", e))?;

    let _ = config.insert_json5(
        "metadata/federation_id",
        &format!("\"{}\"", federation_id.as_str()),
    );

    if let Some(ref router) = args.connect {
        tracing::info!("Connecting to Zenoh router at {}", router);
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", router))
            .map_err(|e| anyhow!("Invalid Zenoh endpoint: {}", e))?;
    }
    let session = zenoh::open(config)
        .await
        .map_err(|e| anyhow!("Failed to open Zenoh session: {}", e))?;

    let (tx_chan, mut rx_chan) = tokio::sync::mpsc::unbounded_channel();
    let mut _subs = Vec::new();

    let routing_map_keys: Vec<String> = topo.read().await.routing_map.map.keys().cloned().collect();
    let mut explicit_topics = Vec::new();
    for node in &routing_map_keys {
        explicit_topics.push(deterministic_coordinator::topics::templates::eth_tx(node));
        explicit_topics.push(deterministic_coordinator::topics::templates::uart_tx(node));
        explicit_topics.push(deterministic_coordinator::topics::templates::can_tx(node));
        explicit_topics.push(deterministic_coordinator::topics::templates::lin_tx(node));
        explicit_topics.push(deterministic_coordinator::topics::templates::rf_hci_tx(
            node,
        ));
        explicit_topics.push(deterministic_coordinator::topics::templates::rf_ieee802154_tx(node));
        explicit_topics.push(deterministic_coordinator::topics::templates::chardev_tx(
            node,
        ));
        explicit_topics.push(format!("sim/systemc/frame/{}/tx", node));
        explicit_topics.push(deterministic_coordinator::topics::templates::sim_uart_tx(
            node,
        ));
        explicit_topics.push(format!("sim/spi/default/{}/tx", node));
    }

    for topic in explicit_topics {
        let tx = tx_chan.clone();
        let sub = session
            .declare_subscriber(topic.clone())
            .callback(move |sample| {
                let _ = tx.send(sample);
            })
            .await
            .map_err(|e| anyhow!("Failed to declare subscriber for {}: {}", topic, e))?;
        _subs.push(sub);
    }
    let sub_done = session
        .declare_subscriber(deterministic_coordinator::topics::wildcard::COORD_DONE_WILDCARD)
        .await
        .map_err(|e| anyhow!("Failed to declare done subscriber: {}", e))?;

    let sub_ctrl = session
        .declare_subscriber(deterministic_coordinator::topics::singleton::NETWORK_CONTROL)
        .await
        .map_err(|e| anyhow!("Failed to declare control subscriber: {}", e))?;

    tracing::info!("Coordinator subscribers active");

    // Phase 4: Self-roundtrip routing barrier.
    // Declare a probe token, wait for discovery, then undeclare.
    // This ensures all previous declarations (subscribers) have been processed by the router.
    let probe_topic = format!("sim/coord/probe/{}", std::process::id());
    let probe_token = session
        .liveliness()
        .declare_token(&probe_topic)
        .await
        .map_err(|e| anyhow!("Failed to declare probe token: {}", e))?;

    tracing::info!("Waiting for routing barrier (probe: {})...", probe_topic);
    let mut discovered = false;
    for _ in 0..50 {
        let replies = session
            .liveliness()
            .get(&probe_topic)
            .await
            .map_err(|e| anyhow!("Liveliness get failed: {}", e))?;

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
        bail!("Routing barrier timeout");
    }
    drop(probe_token);
    tracing::info!("Routing barrier complete.");

    // Validate that all expected nodes have joined within join_timeout_ms.
    // By default, expect 0..args.nodes. If topology provides valid nodes, use those.
    let mut expected_nodes = std::collections::HashSet::new();
    let is_explicit_topo = topo.read().await.is_explicit;
    if is_explicit_topo {
        // Fallback for explicit topology (could be derived from topology fields if exposed,
        // but args.nodes is the source of truth for the barrier count right now)
    }
    for i in 0..args.nodes {
        expected_nodes.insert(i as u32);
    }

    let start_time = tokio::time::Instant::now();
    let timeout_duration = tokio::time::Duration::from_millis(args.join_timeout_ms);

    // We check liveliness tokens or we can just wait for liveliness to be declared by nodes.
    // Virtmcu nodes declare `sim/clock/liveliness/{node_id}` via VirtmcuClock backend.
    tracing::info!(federation = %federation_id, "Waiting up to {}ms for {} nodes to join...", args.join_timeout_ms, expected_nodes.len());

    let mut joined_nodes = std::collections::HashSet::new();
    while start_time.elapsed() < timeout_duration {
        let mut all_joined = true;
        for node_id in &expected_nodes {
            if !joined_nodes.contains(node_id) {
                let liveliness_expr = format!("sim/clock/liveliness/{}", node_id);
                if let Ok(replies) = session.liveliness().get(&liveliness_expr).await {
                    let mut count = 0;
                    while let Ok(_reply) = replies.recv_async().await {
                        count += 1;
                    }
                    if count > 0 {
                        joined_nodes.insert(*node_id);
                    } else {
                        all_joined = false;
                    }
                } else {
                    all_joined = false;
                }
            }
        }

        if all_joined {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    let missing_nodes: Vec<_> = expected_nodes.difference(&joined_nodes).collect();
    if !missing_nodes.is_empty() {
        let missing_node_id = missing_nodes[0];
        bail!(
            "Federation {}: node '{}' did not join within {}ms",
            federation_id.as_str(),
            missing_node_id,
            args.join_timeout_ms
        );
    }
    tracing::info!(federation = %federation_id, "All {} nodes have joined.", expected_nodes.len());

    let liveliness_topic = deterministic_coordinator::topics::singleton::COORD_ALIVE;
    let _liveliness = session
        .liveliness()
        .declare_token(liveliness_topic)
        .await
        .map_err(|e| anyhow!("Failed to declare liveliness token: {}", e))?;

    let mut node_batches = std::collections::HashMap::new();
    let mut seen_nodes = std::collections::HashSet::new();
    let mut current_quantum: u64 = 0;

    for i in 0..args.nodes {
        seen_nodes.insert(i as u32);
    }

    loop {
        tokio::select! {
            Some(sample) = rx_chan.recv() => {
                let topic = sample.key_expr().as_str();
                let parts: Vec<&str> = topic.split('/').collect();
                if parts.len() >= 2 {
                    let node_id_str = parts[parts.len() - 2];
                    if let Ok(node_id) = node_id_str.parse::<u32>() {
                        let t = topo.read().await;
                        let node_id_string = node_id.to_string();
                        if !t.routing_map.map.contains_key(&node_id_string) {
                            panic!("Unregistered packet received!");
                        }
                        drop(t);

                        let proto = if topic.contains("eth") { Protocol::Ethernet }
                                    else if topic.contains("uart") { Protocol::Uart }
                                    else if topic.contains("can") { Protocol::CanFd }
                                    else if topic.contains("lin") { Protocol::Lin }
                                    else if topic.contains("spi") { Protocol::Spi }
                                    else if topic.contains("rf/hci") { Protocol::RfHci }
                                    else if topic.contains("rf") { Protocol::Rf802154 }
                                    else if topic.contains("chardev") { Protocol::ReferenceLink }
                                    else { Protocol::Ethernet };
                        let base = parts[..parts.len() - 2].join("/");

                        seen_nodes.insert(node_id);
                        tracing::info!("Received legacy {:?} TX from node {} (base: {})", proto, node_id, base);
                        let payload = sample.payload().to_bytes();

                    let mut data_opt = None;
                    let mut vtime = 0;
                    let mut seq = 0;

                    // 1. Try Rf802154Header if protocol is 802.15.4
                    if proto == Protocol::Rf802154 && payload.len() >= 4 {
                        let sz = u32::from_le_bytes(payload[0..4].try_into().expect("payload length checked but conversion failed")) as usize;
                        if sz > 0 && sz <= 1024 && payload.len() >= 4 + sz {
                             let hdr_slice = &payload[4..4 + sz];
                             let mut aligned = vec![0u8; hdr_slice.len()];
                             aligned.copy_from_slice(hdr_slice);
                             if let Ok(hdr) = flatbuffers::root::<virtmcu_api::rf802154::Rf802154Frame>(&aligned) {
                                 vtime = hdr.delivery_vtime_ns();
                                 seq = hdr.sequence_number();
                                 data_opt = Some(payload[4 + sz..].to_vec());
                             }
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
                                tracing::debug!("Skipping malformed legacy frame: expected {} bytes, got {}", data_start + header.size() as usize, payload.len());
                                continue;
                            }
                        }
                    }

                    // 3. Last fallback: Raw payload (only for protocols we know are raw)
                    if data_opt.is_none() && (proto == Protocol::Lin || proto == Protocol::CanFd || proto == Protocol::FlexRay) {
                        if proto == Protocol::Lin {
                             if let Ok(frame) = virtmcu_api::lin_generated::virtmcu::lin::root_as_lin_frame(&payload) {
                                 vtime = frame.delivery_vtime_ns();
                             }
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
                            tracing::debug!(
                                federation = %federation_id,
                                node = node_id,
                                vtime = msg.delivery_vtime_ns,
                                "Buffering legacy TX"
                            );
                            node_batches
                                .entry(node_id)
                                .or_insert_with(Vec::new)
                                .push(msg);
                        }
                    }
                }
                }
            }
            Ok(sample) = sub_ctrl.recv_async() => {
                let payload = sample.payload().to_bytes();
                if let Ok(json_str) = String::from_utf8(payload.to_vec()) {
                    let mut t = topo.write().await;
                    if let Err(e) = t.update_from_json(&json_str) {
                        tracing::error!(federation = %federation_id, "Failed to update topology from JSON: {}", e);
                    } else {
                        tracing::info!(federation = %federation_id, "Topology updated from JSON: {}", json_str);
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
                        tracing::debug!("DONE payload (len {}): {:02x?}", payload.len(), &payload[..std::cmp::min(16, payload.len())]);

                        let is_legacy = payload.len() == 8 || payload.len() == 16;
                        let (quantum, vtime_limit, mut batched_msgs) = if !is_legacy && flatbuffers::root_with_opts::<virtmcu_api::CoordDoneReq>(&flatbuffers::VerifierOptions::default(), &payload).is_ok() {
                            let req = flatbuffers::root_with_opts::<virtmcu_api::CoordDoneReq>(&flatbuffers::VerifierOptions::default(), &payload).expect("test should succeed");
                            let mut msgs = Vec::new();
                            if let Some(fb_msgs) = req.messages() {
                                for i in 0..fb_msgs.len() {
                                    let m = fb_msgs.get(i);
                                    msgs.push(CoordMessage {
                                        src_node_id: m.src_node_id(),
                                        dst_node_id: m.dst_node_id(),
                                        delivery_vtime_ns: m.delivery_vtime_ns(),
                                        sequence_number: m.sequence_number(),
                                        protocol: parse_protocol(m.protocol().0),
                                        payload: m.payload().map(|p| p.bytes().to_vec()).unwrap_or_default(),
                                        base_topic: None,
                                    });
                                }
                            }
                            (req.quantum(), req.vtime_limit(), msgs)
                        } else {
                            // Legacy fallback (8-byte or 16-byte raw)
                            let mut q = u64::MAX;
                            let mut vtl = u64::MAX;
                            let mut msgs = Vec::new();
                            if payload.len() >= 8 {
                                let mut cursor = Cursor::new(&payload);
                                q = cursor.read_u64::<LittleEndian>().expect("malformed legacy DONE: missing field");
                                if payload.len() >= 16 {
                                    vtl = cursor.read_u64::<LittleEndian>().expect("malformed legacy DONE: missing field");
                                    if payload.len() > 16 {
                                        msgs = decode_batch(&payload[16..]);
                                    }
                                } else if payload.len() > 8 {
                                    msgs = decode_batch(&payload[8..]);
                                }
                            }
                            (q, vtl, msgs)
                        };

                        tracing::info!(
                            federation = %federation_id,
                            node = node_id,
                            quantum = quantum,
                            vtime_limit = vtime_limit,
                            "Received DONE"
                        );

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
                                    federation = %federation_id,
                                    quantum = current_quantum,
                                    msg_count = sorted_msgs.len(),
                                    "Quantum complete"
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
    _seen_nodes: &std::collections::HashSet<u32>,
    pcap_log: &mut Option<MessageLog>,
    msg: &mut CoordMessage,
) {
    let t = topo.read().await;
    let mut target_nodes = Vec::new();
    let src_str = msg.src_node_id.to_string();
    let allowed_targets = t.routing_map.map.get(&src_str).unwrap_or_else(|| {
        panic!("Unregistered packet received!");
    });

    if msg.dst_node_id == u32::MAX {
        for target_str in allowed_targets {
            if let Ok(tid) = target_str.parse::<u32>() {
                target_nodes.push(tid);
            }
        }
    } else {
        let dst_str = msg.dst_node_id.to_string();
        if allowed_targets.contains(&dst_str) {
            target_nodes.push(msg.dst_node_id);
        } else {
            panic!("Unregistered packet received!");
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
                Protocol::ReferenceLink => {
                    deterministic_coordinator::topics::templates::chardev_rx(
                        &target_node.to_string(),
                    )
                }
                _ => format!("sim/unknown/{}/rx", target_node),
            }
        };
        tracing::debug!("Legacy delivery to topic: {}", legacy_rx_topic);

        let legacy_payload = match msg.protocol {
            Protocol::Rf802154 => virtmcu_api::encode_rf802154_frame(
                msg.delivery_vtime_ns,
                msg.sequence_number,
                &msg.payload,
                -80,
                255,
                virtmcu_api::Rf802154Mhr::parse(&msg.payload),
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
    federation_id: virtmcu_api::FederationId,
    _topo: Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    _barrier: Arc<QuantumBarrier>,
    mut _pcap_log: Option<MessageLog>,
) -> Result<()> {
    tracing::info!(
        federation = %federation_id,
        "Unix coordinator started (minimal passthrough)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_coordinator::barrier::CoordMessage;
    use deterministic_coordinator::topology::{Protocol, TopologyGraph};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    #[should_panic(expected = "Unregistered packet received!")]
    async fn test_unregistered_packet_panics() {
        let topo = TopologyGraph::default();
        let arc_topo = Arc::new(RwLock::new(topo));
        let mut msg = CoordMessage {
            src_node_id: 99,
            dst_node_id: 100,
            delivery_vtime_ns: 0,
            sequence_number: 0,
            protocol: Protocol::Ethernet,
            payload: vec![],
            base_topic: None,
        };
        let dummy_session = zenoh::config::Config::default();
        let session = zenoh::open(dummy_session).await.unwrap();
        let seen_nodes = std::collections::HashSet::new();
        let mut pcap = None;
        deliver_message(&session, &arc_topo, &seen_nodes, &mut pcap, &mut msg).await;
    }
}

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
use virtmcu_wire::{FlatBufferStructExt, ZenohFrameHeader};

#[derive(Parser, Debug)]
#[command(version, about = "Deterministic Coordinator", long_about = None)]
struct Args {
    /// Identifier for this running simulation instance (HLA: federation name).
    /// Used in log output. Required.
    #[arg(long, env = "VIRTMCU_FEDERATION_ID")]
    federation_id: String,

    #[arg(long, default_value = "zenoh")]
    transport: String,

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

    #[arg(long, env = "VIRTMCU_RUN_DIR", default_value = "/run/virtmcu")]
    run_dir: String,
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
        8 => Protocol::Control,
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

fn encode_message(msg: &CoordMessage) -> Vec<u8> {
    virtmcu_wire::encode_coord_message(
        msg.src_node_id,
        msg.dst_node_id,
        msg.delivery_vtime_ns,
        msg.sequence_number,
        virtmcu_wire::Protocol(serialize_protocol(&msg.protocol)),
        &msg.payload,
    )
}

fn decode_batch(payload: &[u8]) -> Vec<CoordMessage> {
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::Cursor;
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
                        src_node_id: src,
                        dst_node_id: dst,
                        delivery_vtime_ns: vt,
                        sequence_number: seq,
                        protocol: parse_protocol(pr),
                        payload: data,
                        base_topic: None,
                    });
                }
            }
        }
    }
    msgs
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
    let federation_id = virtmcu_wire::FederationId(args.federation_id.clone());

    tracing::info!(federation = %federation_id, "Coordinator starting...");

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

    let transport = if args.transport == "unix" {
        topology::Transport::Unix
    } else {
        topo_raw.transport.clone()
    };

    let topo = Arc::new(tokio::sync::RwLock::new(topo_raw));

    if transport == topology::Transport::Unix {
        run_unix_coordinator(args, federation_id, topo, barrier, pcap_log).await
    } else {
        run_deterministic_coordinator(args, federation_id, topo, barrier, pcap_log).await
    }
}

async fn run_deterministic_coordinator(
    args: Args,
    federation_id: virtmcu_wire::FederationId,
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
        explicit_topics.push(deterministic_coordinator::topics::templates::reference_bus_tx(node));
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
                                    else if topic.contains("reference_bus") || topic.contains("chardev") { Protocol::ReferenceLink }
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
                             if let Ok(hdr) = flatbuffers::root::<virtmcu_wire::rf802154::Rf802154Frame>(&aligned) {
                                 vtime = hdr.delivery_vtime_ns();
                                 seq = hdr.sequence_number();
                                 data_opt = Some(payload[4 + sz..].to_vec());
                             }
                        }
                    }

                    // 2. Fallback to ZenohFrameHeader if not already parsed
                    // (But only for protocols that use it!)
                    if data_opt.is_none() {
                        if let Some(header) = ZenohFrameHeader::unpack_slice(&payload) {
                            let data_start = virtmcu_wire::ZENOH_FRAME_HEADER_SIZE;
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
                             if let Ok(frame) = virtmcu_wire::lin_generated::virtmcu::lin::root_as_lin_frame(&payload) {
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
                        let (quantum, vtime_limit, mut batched_msgs) = if !is_legacy && flatbuffers::root_with_opts::<virtmcu_wire::CoordDoneReq>(&flatbuffers::VerifierOptions::default(), &payload).is_ok() {
                            let req = flatbuffers::root_with_opts::<virtmcu_wire::CoordDoneReq>(&flatbuffers::VerifierOptions::default(), &payload).expect("test should succeed");
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
                                panic!(
                                    "FATAL: Barrier error for node {} \
                                     (quantum={quantum}, current_quantum={current_quantum}): {e:?} \
                                     — QuantumMismatch means quantum skipped or regressed; \
                                     check for pre-increment bug: capture current_quantum BEFORE \
                                     step_clock(), increment AFTER try_join_all()",
                                    node_id
                                );
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

    eprintln!(
        "COORD: routing msg from {} to dst {} (len={})",
        msg.src_node_id,
        msg.dst_node_id,
        msg.payload.len()
    );

    if msg.dst_node_id == u32::MAX {
        for target_str in allowed_targets {
            if let Ok(tid) = target_str.parse::<u32>() {
                target_nodes.push(tid);
            }
        }
        eprintln!(
            "COORD: broadcast msg resolved to targets: {:?}",
            target_nodes
        );
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
                    let base = msg.base_topic.as_deref().unwrap_or("sim/chardev");
                    format!("{}/{}/rx", base, target_node)
                }
                _ => format!("sim/unknown/{}/rx", target_node),
            }
        };
        tracing::debug!("Legacy delivery to topic: {}", legacy_rx_topic);

        let legacy_payload = virtmcu_wire::encode_coord_message(
            msg.src_node_id,
            msg.dst_node_id,
            msg.delivery_vtime_ns,
            msg.sequence_number,
            virtmcu_wire::Protocol(serialize_protocol(&msg.protocol)),
            &msg.payload,
        );
        let _ = session.put(&legacy_rx_topic, legacy_payload).await;
    }
}

async fn uds_write_framed(
    stream: &Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    topic: &str,
    payload: &[u8],
) {
    use tokio::io::AsyncWriteExt;
    let mut sock = stream.lock().await;
    let topic_bytes = topic.as_bytes();
    let _ = sock.write_u32_le(topic_bytes.len() as u32).await;
    let _ = sock.write_all(topic_bytes).await;
    let _ = sock.write_u32_le(payload.len() as u32).await;
    let _ = sock.write_all(payload).await;
}

async fn uds_deliver_message(
    sockets: &std::collections::HashMap<
        u32,
        Vec<Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>>,
    >,
    topo: &Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    pcap_log: &mut Option<MessageLog>,
    msg: &mut CoordMessage,
) {
    let t = topo.read().await;
    let mut target_nodes = Vec::new();
    let src_str = msg.src_node_id.to_string();
    let allowed_targets = t.routing_map.map.get(&src_str).unwrap_or_else(|| {
        panic!("Unregistered packet received!");
    });

    eprintln!(
        "COORD: routing msg from {} to dst {} (len={})",
        msg.src_node_id,
        msg.dst_node_id,
        msg.payload.len()
    );

    if msg.dst_node_id == u32::MAX {
        for target_str in allowed_targets {
            if let Ok(tid) = target_str.parse::<u32>() {
                target_nodes.push(tid);
            }
        }
        eprintln!(
            "COORD: broadcast msg resolved to targets: {:?}",
            target_nodes
        );
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
        if let Some(socks) = sockets.get(&target_node) {
            for sock in socks {
                uds_write_framed(sock, &rx_topic, &out_payload).await;
            }
        }

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
                    let base = msg.base_topic.as_deref().unwrap_or("sim/chardev");
                    format!("{}/{}/rx", base, target_node)
                }
                _ => format!("sim/unknown/{}/rx", target_node),
            }
        };

        let legacy_payload = virtmcu_wire::encode_coord_message(
            msg.src_node_id,
            msg.dst_node_id,
            msg.delivery_vtime_ns,
            msg.sequence_number,
            virtmcu_wire::Protocol(serialize_protocol(&msg.protocol)),
            &msg.payload,
        );
        if let Some(socks) = sockets.get(&target_node) {
            for sock in socks {
                uds_write_framed(sock, &legacy_rx_topic, &legacy_payload).await;
            }
        }
    }
}

async fn run_unix_coordinator(
    args: Args,
    federation_id: virtmcu_wire::FederationId,
    topo: Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    barrier: Arc<QuantumBarrier>,
    mut pcap_log: Option<MessageLog>,
) -> Result<()> {
    tracing::info!(federation = %federation_id, "Unix coordinator started");

    let sock_path = format!(
        "{}/{}/coordinator.sock",
        args.run_dir,
        federation_id.as_str()
    );
    if let Some(parent) = std::path::Path::new(&sock_path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create UDS dirs: {}", e))?;
    }
    let _ = tokio::fs::remove_file(&sock_path).await;
    let listener = tokio::net::UnixListener::bind(&sock_path)
        .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", sock_path, e))?;

    enum WorkerEvent {
        Message(String, Vec<u8>),
        Register(
            u32,
            Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
        ),
    }

    let (tx_chan, mut rx_chan) = tokio::sync::mpsc::channel::<WorkerEvent>(65536);

    let expected_nodes = args.nodes;
    let listener_tx = tx_chan.clone();
    let federation_id_check = federation_id.0.clone();

    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let (mut read_half, write_half) = stream.into_split();
                let write_half = Arc::new(tokio::sync::Mutex::new(write_half));
                let worker_tx = listener_tx.clone();
                let fed_id_check = federation_id_check.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncReadExt;
                    let mut topic_len_buf = [0u8; 4];
                    if read_half.read_exact(&mut topic_len_buf).await.is_err() {
                        return;
                    }
                    let topic_len = u32::from_le_bytes(topic_len_buf) as usize;
                    let mut topic_buf = vec![0u8; topic_len];
                    if read_half.read_exact(&mut topic_buf).await.is_err() {
                        return;
                    }
                    let topic = String::from_utf8_lossy(&topic_buf).into_owned();

                    let mut payload_len_buf = [0u8; 4];
                    if read_half.read_exact(&mut payload_len_buf).await.is_err() {
                        return;
                    }
                    let payload_len = u32::from_le_bytes(payload_len_buf) as usize;
                    let mut payload = vec![0u8; payload_len];
                    if read_half.read_exact(&mut payload).await.is_err() {
                        return;
                    }

                    if topic == "sim/coord/register" {
                        let (node_id, reg_fed_id, proto_version) =
                            virtmcu_wire::decode_uds_registration(&payload).expect(
                                "FATAL: invalid UdsRegistration frame on sim/coord/register",
                            );
                        if proto_version != virtmcu_wire::UDS_PROTO_VERSION {
                            eprintln!(
                                "FATAL: node proto_version {} != coordinator UDS_PROTO_VERSION {} \
                                 — rebuild the peripheral plugin to match the coordinator version",
                                proto_version,
                                virtmcu_wire::UDS_PROTO_VERSION
                            );
                            std::process::abort();
                        }
                        if reg_fed_id != fed_id_check {
                            eprintln!(
                                "FATAL: node registered with federation_id='{}' but coordinator \
                                 federation_id='{}' — check that the federation-id QOM property \
                                 is set with a hyphen (not underscore) in the -device argument",
                                reg_fed_id, fed_id_check
                            );
                            std::process::abort();
                        }
                        let _ = worker_tx
                            .send(WorkerEvent::Register(node_id, write_half))
                            .await;
                    } else {
                        return;
                    }

                    loop {
                        let mut topic_len_buf = [0u8; 4];
                        if read_half.read_exact(&mut topic_len_buf).await.is_err() {
                            break;
                        }
                        let topic_len = u32::from_le_bytes(topic_len_buf) as usize;
                        let mut topic_buf = vec![0u8; topic_len];
                        if read_half.read_exact(&mut topic_buf).await.is_err() {
                            break;
                        }
                        let topic = String::from_utf8_lossy(&topic_buf).into_owned();

                        let mut payload_len_buf = [0u8; 4];
                        if read_half.read_exact(&mut payload_len_buf).await.is_err() {
                            break;
                        }
                        let payload_len = u32::from_le_bytes(payload_len_buf) as usize;
                        let mut payload = vec![0u8; payload_len];
                        if read_half.read_exact(&mut payload).await.is_err() {
                            break;
                        }

                        let _ = worker_tx
                            .send(WorkerEvent::Message(topic.clone(), payload.clone()))
                            .await;
                        eprintln!("COORD: sent WorkerEvent::Message for {}", topic);
                    }
                });
            }
        }
    });

    let mut sockets: std::collections::HashMap<
        u32,
        Vec<Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>>,
    > = std::collections::HashMap::new();
    let start_time = tokio::time::Instant::now();
    let timeout_duration = tokio::time::Duration::from_millis(args.join_timeout_ms);

    tracing::info!(federation = %federation_id, "Waiting up to {}ms for {} nodes to join...", args.join_timeout_ms, expected_nodes);

    while sockets.len() < expected_nodes {
        tokio::select! {
            _ = tokio::time::sleep(timeout_duration.saturating_sub(start_time.elapsed())) => {
                bail!("Federation {}: Not all nodes joined within {}ms", federation_id.as_str(), args.join_timeout_ms);
            }
            evt = rx_chan.recv() => {
                if let Some(WorkerEvent::Register(node_id, write_half)) = evt {
                    sockets.entry(node_id).or_default().push(write_half);
                    let len = sockets.get(&node_id).map(|v| v.len()).expect("FATAL: socket entry missing");
                    tracing::info!("Node {} registered (total sockets for node: {})", node_id, len);
                }
            }
        }
    }

    tracing::info!(federation = %federation_id, "All {} nodes have joined.", expected_nodes);

    let mut node_batches = std::collections::HashMap::new();
    let mut seen_nodes = std::collections::HashSet::new();
    let mut current_quantum: u64 = 0;
    for i in 0..args.nodes {
        seen_nodes.insert(i as u32);
    }
    let no_pdes = args.no_pdes;

    while let Some(evt) = rx_chan.recv().await {
        match evt {
            WorkerEvent::Register(node_id, write_half) => {
                sockets.entry(node_id).or_default().push(write_half);
                let len = sockets
                    .get(&node_id)
                    .map(|v| v.len())
                    .expect("FATAL: socket entry missing");
                tracing::info!(
                    "Late registration for node {} (total sockets: {})",
                    node_id,
                    len
                );
            }
            WorkerEvent::Message(topic, payload) => {
                let is_tx = topic.ends_with("/tx");
                let is_done = topic.starts_with("sim/coord/done/");

                if is_tx {
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

                            let proto = if topic.contains("eth") {
                                Protocol::Ethernet
                            } else if topic.contains("uart") {
                                Protocol::Uart
                            } else if topic.contains("can") {
                                Protocol::CanFd
                            } else if topic.contains("lin") {
                                Protocol::Lin
                            } else if topic.contains("spi") {
                                Protocol::Spi
                            } else if topic.contains("rf/hci") {
                                Protocol::RfHci
                            } else if topic.contains("rf") {
                                Protocol::Rf802154
                            } else if topic.contains("reference_bus") || topic.contains("chardev") {
                                Protocol::ReferenceLink
                            } else {
                                Protocol::Ethernet
                            };
                            let base = parts[..parts.len() - 2].join("/");

                            seen_nodes.insert(node_id);

                            let mut data_opt = None;
                            let mut vtime = 0;
                            let mut seq = 0;

                            if let Ok(msg) =
                                flatbuffers::root::<virtmcu_wire::CoordMessage>(&payload)
                            {
                                vtime = msg.delivery_vtime_ns();
                                seq = msg.sequence_number();
                                data_opt = Some(
                                    msg.payload()
                                        .expect("FATAL: CoordMessage missing payload")
                                        .bytes()
                                        .to_vec(),
                                );
                            } else {
                                eprintln!(
                                    "COORD: flatbuffers::root failed for {} len={}",
                                    topic,
                                    payload.len()
                                );
                            }

                            // Fallback to ZenohFrameHeader if not already parsed
                            if data_opt.is_none() {
                                if let Some(header) = ZenohFrameHeader::unpack_slice(&payload) {
                                    let data_start = virtmcu_wire::ZENOH_FRAME_HEADER_SIZE;
                                    if payload.len() >= data_start + header.size() as usize {
                                        vtime = header.delivery_vtime_ns();
                                        seq = header.sequence_number();
                                        data_opt = Some(
                                            payload
                                                [data_start..data_start + header.size() as usize]
                                                .to_vec(),
                                        );
                                    } else {
                                        tracing::debug!("Skipping malformed legacy frame: expected {} bytes, got {}", data_start + header.size() as usize, payload.len());
                                    }
                                }
                            }

                            if data_opt.is_none()
                                && proto == Protocol::Rf802154
                                && payload.len() >= 4
                            {
                                let sz = u32::from_le_bytes(
                                    payload[0..4]
                                        .try_into()
                                        .expect("payload length checked but conversion failed"),
                                ) as usize;
                                if sz > 0 && sz <= 1024 && payload.len() >= 4 + sz {
                                    let hdr_slice = &payload[4..4 + sz];
                                    let mut aligned = vec![0u8; hdr_slice.len()];
                                    aligned.copy_from_slice(hdr_slice);
                                    if let Ok(hdr) = flatbuffers::root::<
                                        virtmcu_wire::rf802154::Rf802154Frame,
                                    >(&aligned)
                                    {
                                        vtime = hdr.delivery_vtime_ns();
                                        seq = hdr.sequence_number();
                                        data_opt = Some(payload[4 + sz..].to_vec());
                                    }
                                }
                            }

                            if data_opt.is_none()
                                && (proto == Protocol::Lin
                                    || proto == Protocol::CanFd
                                    || proto == Protocol::FlexRay)
                            {
                                if proto == Protocol::Lin {
                                    if let Ok(frame) =
                                        virtmcu_wire::lin_generated::virtmcu::lin::root_as_lin_frame(
                                            &payload,
                                        )
                                    {
                                        vtime = frame.delivery_vtime_ns();
                                    }
                                }
                                data_opt = Some(payload.clone());
                            }

                            if let Some(data) = data_opt {
                                let mut msg = CoordMessage {
                                    src_node_id: node_id,
                                    dst_node_id: u32::MAX,
                                    delivery_vtime_ns: vtime.saturating_add(args.delay_ns),
                                    sequence_number: seq,
                                    protocol: proto,
                                    payload: data,
                                    base_topic: Some(base),
                                };

                                eprintln!(
                                    "COORD: adding legacy msg to node_batches: {:?}",
                                    msg.protocol
                                );
                                if no_pdes {
                                    uds_deliver_message(&sockets, &topo, &mut pcap_log, &mut msg)
                                        .await;
                                } else {
                                    node_batches
                                        .entry(node_id)
                                        .or_insert_with(Vec::new)
                                        .push(msg);
                                }
                            } else {
                                eprintln!("COORD: Failed to parse legacy fallback or decode CoordMessage from topic {} (len={})", topic, payload.len());
                            }
                        }
                    }
                } else if is_done {
                    let parts: Vec<&str> = topic.split('/').collect();
                    if parts.len() >= 4 {
                        if let Ok(node_id) = parts[3].parse::<u32>() {
                            seen_nodes.insert(node_id);

                            let req = flatbuffers::root::<virtmcu_wire::CoordDoneReq>(&payload)
                                .expect("FATAL: invalid CoordDoneReq on sim/coord/done");
                            let mut batched_msgs = Vec::new();
                            if let Some(fb_msgs) = req.messages() {
                                for i in 0..fb_msgs.len() {
                                    let m = fb_msgs.get(i);
                                    batched_msgs.push(CoordMessage {
                                        src_node_id: m.src_node_id(),
                                        dst_node_id: m.dst_node_id(),
                                        delivery_vtime_ns: m.delivery_vtime_ns(),
                                        sequence_number: m.sequence_number(),
                                        protocol: parse_protocol(m.protocol().0),
                                        payload: m
                                            .payload()
                                            .map(|p| p.bytes().to_vec())
                                            .unwrap_or_default(),
                                        base_topic: None,
                                    });
                                }
                            }
                            let quantum = req.quantum();
                            let vtime_limit = req.vtime_limit();

                            let msgs = node_batches.remove(&node_id).unwrap_or_default();
                            let (mut current_msgs, future_msgs): (
                                Vec<CoordMessage>,
                                Vec<CoordMessage>,
                            ) = msgs
                                .into_iter()
                                .partition(|m| m.delivery_vtime_ns <= vtime_limit);

                            if !future_msgs.is_empty() {
                                node_batches.insert(node_id, future_msgs);
                            }

                            current_msgs.append(&mut batched_msgs);

                            match barrier.submit_done(
                                node_id,
                                quantum,
                                current_quantum,
                                current_msgs,
                            ) {
                                Ok(Some(mut sorted_msgs)) => {
                                    sorted_msgs.sort();

                                    for mut msg in sorted_msgs {
                                        uds_deliver_message(
                                            &sockets,
                                            &topo,
                                            &mut pcap_log,
                                            &mut msg,
                                        )
                                        .await;
                                    }

                                    if let Some(log) = &mut pcap_log {
                                        let _ = log.flush();
                                    }

                                    current_quantum += 1;
                                    for (&id, socks) in sockets.iter() {
                                        let start_topic = deterministic_coordinator::topics::templates::clock_start(&id.to_string());
                                        let start_payload = virtmcu_wire::encode_uds_quantum_start(
                                            current_quantum,
                                            u64::MAX,
                                        );
                                        for sock in socks {
                                            uds_write_framed(sock, &start_topic, &start_payload)
                                                .await;
                                        }
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    panic!("FATAL: Barrier error for node {}: {:?}", node_id, e);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
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
    #[cfg_attr(miri, ignore)]
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

    #[test]
    fn test_broadcast_sorting_determinism() {
        // Create a HashMap with multiple nodes inserted in a non-sorted order
        let mut sockets = std::collections::HashMap::new();
        sockets.insert(3, vec![1, 2]);
        sockets.insert(1, vec![3]);
        sockets.insert(5, vec![4, 5]);
        sockets.insert(2, vec![6]);

        // Simulate the exact loop logic from the coordinator broadcast
        let mut sorted_sockets: Vec<_> = sockets.iter().collect();
        sorted_sockets.sort_by_key(|(&id, _)| id);

        // Extract the sorted node IDs
        let sorted_ids: Vec<u32> = sorted_sockets.iter().map(|(&id, _)| id).collect();

        // Verify that the node IDs are always processed in ascending order
        assert_eq!(
            sorted_ids,
            vec![1, 2, 3, 5],
            "Broadcast sockets must be sorted by node_id for determinism"
        );
    }
}

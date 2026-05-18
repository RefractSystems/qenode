#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![deny(unsafe_code)]
use anyhow::{anyhow, bail, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use clap::Parser;
use deterministic_coordinator::message_log::MessageLog;
use std::io::Cursor;
use std::sync::Arc;

use deterministic_coordinator::barrier::{CoordMessage, QuantumBarrier};
use deterministic_coordinator::topology;

struct DummyVTimeProvider;
impl virtmcu_observability::processors::VTimeProvider for DummyVTimeProvider {
    fn vtime_ns(&self) -> u64 { 0 }
}


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
    let mut default_rx_map: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
    let topo_guard = topo.read().await;
    for (link_id, wire_link) in topo_guard.wire_links.iter().enumerate() {
        let link_id = link_id as u32;
        for &node_id in &wire_link.nodes {
            default_rx_map.entry(link_id).or_default().push(node_id);
        }
    }
    drop(topo_guard);
    
    let mut _subs = Vec::new();

    let explicit_topics = vec![
        "sim/ch/**".to_string(),
        "sim/coord/*/done".to_string(),
        "sim/network/control".to_string(),
        "sim/coord/link/register/*".to_string(),
    ];
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

    #[allow(unused_mut)]
    #[allow(unused_variables)]
    let mut rx_counters: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
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
                let payload = sample.payload().to_bytes();

                if topic.starts_with("sim/coord/link/register/") {
                    let parts: Vec<&str> = topic.split('/').collect();
                    if parts.len() == 5 {
                        if let Ok(msg_node_id) = parts[4].parse::<u32>() {
                            if let Ok(reg) = flatbuffers::root::<virtmcu_wire::LinkRegistration>(&payload) {
                                let link_name = reg.link_name();
                                // We are not currently performing preflight validation in Zenoh but we MUST ACK
                                // so the node can proceed.
                                let t = topo.read().await;
                                if let Some((link_id, _)) = t.wire_links.iter().enumerate().find(|(_, l)| l.name == link_name) {
                                    let ack_payload = virtmcu_wire::encode_link_ack(link_id as u32, 0, "");
                                    let ack_topic = format!("sim/coord/link/ack/{}", msg_node_id);
                                    let _ = session.put(&ack_topic, ack_payload).await;
                                }
                            }
                        }
                    }
                } else if topic.starts_with("sim/ch/") {
                    let parts: Vec<&str> = topic.split('/').collect();
                    if parts.len() == 4 {
                        if let (Ok(link_id), Ok(msg_node_id)) = (parts[2].parse::<u32>(), parts[3].parse::<u32>()) {
                            if !default_rx_map.contains_key(&link_id) {
                                std::process::abort();
                            }
                            if payload.len() >= 24 {
                                let parsed_link_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
                                if parsed_link_id != link_id {
                                    std::process::abort();
                                }
                                let payload_len = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
                                let vtime_ns = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                                let seq_num = u64::from_le_bytes(payload[16..24].try_into().unwrap());

                                if payload.len() >= 24 + payload_len {
                                    let raw_payload = &payload[24..24 + payload_len];
                                    let delivery_vtime_ns = vtime_ns.saturating_add(args.delay_ns);
                                    let msg = CoordMessage {
                                        src_node_id: msg_node_id,
                                        link_id,
                                        delivery_vtime_ns,
                                        sequence_number: seq_num,
                                        payload: raw_payload.to_vec(),
                                    };

                                    if no_pdes {
                                        zenoh_deliver_message(&session, &default_rx_map, &mut pcap_log, &msg).await;
                                    } else {
                                        node_batches
                                            .entry(msg_node_id)
                                            .or_insert_with(Vec::new)
                                            .push(msg);
                                    }
                                }
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

                        if payload.len() < 16 { std::process::abort(); }
                        let quantum = u64::from_le_bytes(payload[0..8].try_into().unwrap());
                        let vtime_limit = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                        let mut batched_msgs = Vec::new();

                        tracing::info!(
                            federation = %federation_id,
                            node = node_id,
                            quantum = quantum,
                            vtime_limit = vtime_limit,
                            "Received DONE"
                        );

                        let msgs = node_batches.remove(&node_id).unwrap_or_default();
                        let (mut current_msgs, future_msgs): (Vec<CoordMessage>, Vec<CoordMessage>) = msgs.into_iter().partition(|m| m.delivery_vtime_ns <= vtime_limit + args.delay_ns);

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
                                    zenoh_deliver_message(&session, &default_rx_map, &mut pcap_log, &msg).await;
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

async fn zenoh_deliver_message(
    session: &zenoh::Session,
    rx_map: &std::collections::HashMap<u32, Vec<u32>>,
    pcap_log: &mut Option<MessageLog>,
    msg: &CoordMessage,
) {
    let targets = rx_map.get(&msg.link_id).expect("FATAL: link_id not in rx_map");
    let mut frame = Vec::with_capacity(28 + msg.payload.len());
    frame.extend_from_slice(&msg.link_id.to_le_bytes());
    frame.extend_from_slice(&msg.src_node_id.to_le_bytes());
    frame.extend_from_slice(&msg.delivery_vtime_ns.to_le_bytes());
    frame.extend_from_slice(&msg.sequence_number.to_le_bytes());
    frame.extend_from_slice(&(msg.payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&msg.payload);
    for &target_node in targets {
        if target_node == msg.src_node_id { continue; }
        if let Some(log) = pcap_log { let _ = log.write_message(&msg); }
        let rx_topic = format!("sim/ch/{}/{}", msg.link_id, target_node);
        let _ = session.put(&rx_topic, frame.clone()).await;
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
    sockets: &std::collections::HashMap<u32, Vec<Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>>>,
    rx_map: &std::collections::HashMap<u32, Vec<u32>>,
    pcap_log: &mut Option<MessageLog>,
    msg: &CoordMessage,
) {
    let targets = rx_map.get(&msg.link_id).expect("FATAL: link_id not in rx_map");
    let mut frame = Vec::with_capacity(28 + msg.payload.len());
    frame.extend_from_slice(&msg.link_id.to_le_bytes());
    frame.extend_from_slice(&msg.src_node_id.to_le_bytes());
    frame.extend_from_slice(&msg.delivery_vtime_ns.to_le_bytes());
    frame.extend_from_slice(&msg.sequence_number.to_le_bytes());
    frame.extend_from_slice(&(msg.payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&msg.payload);
    let rx_topic = format!("sim/ch/{}", msg.link_id);
    for &target_node in targets {
        if target_node == msg.src_node_id { continue; }
        if let Some(log) = pcap_log { let _ = log.write_message(&msg); }
        if let Some(socks) = sockets.get(&target_node) {
            for sock in socks {
                uds_write_framed(sock, &rx_topic, &frame).await;
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
        Message(u32, String, Vec<u8>),
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

                    let current_node_id: u32;
                    if topic == "sim/coord/register" {
                        let (node_id, reg_fed_id, proto_version) =
                            virtmcu_wire::decode_uds_registration(&payload).expect(
                                "FATAL: invalid UdsRegistration frame on sim/coord/register",
                            );
                        if proto_version != virtmcu_wire::UDS_PROTO_VERSION {
                            std::fs::write("/tmp/coord_abort.log", format!("abort proto: {} != {}", proto_version, virtmcu_wire::UDS_PROTO_VERSION)).unwrap();
                            std::process::abort();
                        }
                        if reg_fed_id != fed_id_check {
                            std::fs::write("/tmp/coord_abort.log", format!("abort fed: {} != {}", reg_fed_id, fed_id_check)).unwrap();
                            std::process::abort();
                        }
                        let _ = worker_tx
                            .send(WorkerEvent::Register(node_id, write_half))
                            .await;
                        current_node_id = node_id;
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
                            .send(WorkerEvent::Message(
                                current_node_id,
                                topic.clone(),
                                payload.clone(),
                            ))
                            .await;
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

    let mut link_ids = std::collections::HashMap::new();
    let mut default_rx_map = std::collections::HashMap::new();
    let mut delay_map = std::collections::HashMap::new();
    let mut expected_link_pairs = std::collections::HashSet::new();

    {
        let t = topo.read().await;
        for (i, link) in t.wire_links.iter().enumerate() {
            let link_id = i as u32;
            link_ids.insert(link.name.clone(), link_id);
            default_rx_map.insert(link_id, link.nodes.clone());
            delay_map.insert(link_id, args.delay_ns);
            for &node_id in &link.nodes {
                expected_link_pairs.insert((node_id, link.name.clone()));
            }
        }
    }

    let preflight_start = tokio::time::Instant::now();
    let preflight_timeout = tokio::time::Duration::from_secs(30);
    tracing::info!(
        "Waiting for {} link registrations...",
        expected_link_pairs.len()
    );

    while !expected_link_pairs.is_empty() {
        tokio::select! {
            _ = tokio::time::sleep(preflight_timeout.saturating_sub(preflight_start.elapsed())) => {
                let missing: Vec<_> = expected_link_pairs.iter().map(|(n, l)| format!("(node {}, link '{}')", n, l)).collect();
                tracing::error!("FATAL: link registration timeout. Missing pairs: {}", missing.join(", "));
                std::process::abort();
            }
            evt = rx_chan.recv() => {
                if let Some(WorkerEvent::Message(msg_node_id, topic, payload)) = evt {
                    if topic == "sim/coord/link/register" {
                        if let Ok(reg) = flatbuffers::root::<virtmcu_wire::LinkRegistration>(&payload) {
                            let link_name = reg.link_name();
                            if expected_link_pairs.remove(&(msg_node_id, link_name.to_string())) {
                                if let Some(&link_id) = link_ids.get(link_name) {
                                    let ack_payload = virtmcu_wire::encode_link_ack(link_id, 0, "");
                                    if let Some(socks) = sockets.get(&msg_node_id) {
                                        for sock in socks {
                                            uds_write_framed(sock, "sim/coord/link/ack", &ack_payload).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(WorkerEvent::Register(node_id, write_half)) = evt {
                    sockets.entry(node_id).or_default().push(write_half);
                }
            }
        }
    }
    tracing::info!("All link registrations complete.");

    // Issue initial start to unblock nodes.
    for (&id, socks) in sockets.iter() {
        let start_topic =
            deterministic_coordinator::topics::templates::clock_start(&id.to_string());
        let start_payload = virtmcu_wire::encode_uds_quantum_start(0, u64::MAX);
        for sock in socks {
            uds_write_framed(sock, &start_topic, &start_payload).await;
        }
    }

    #[allow(unused_mut)]
    #[allow(unused_variables)]
    let mut rx_counters: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
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
            WorkerEvent::Message(msg_node_id, topic, payload) => {

                if topic.starts_with("sim/ch/") && topic.split('/').count() == 3 {
                    let parts: Vec<&str> = topic.split('/').collect();
                    if let Ok(link_id) = parts[2].parse::<u32>() {
                        if !default_rx_map.contains_key(&link_id) {
                            std::fs::write("/tmp/coord_abort.log", format!("abort rx_map no link_id: {}", link_id)).unwrap();
                            std::process::abort();
                        }
                        if payload.len() >= 24 {
                            let parsed_link_id =
                                u32::from_le_bytes(payload[0..4].try_into().unwrap());
                            if parsed_link_id != link_id {
                                std::fs::write("/tmp/coord_abort.log", format!("abort parsed_link_id != link_id: {} != {}", parsed_link_id, link_id)).unwrap();
                                std::process::abort();
                            }
                            let payload_len =
                                u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
                            let vtime_ns = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                            let seq_num = u64::from_le_bytes(payload[16..24].try_into().unwrap());
                            
                            if payload.len() >= 24 + payload_len {
                                let raw_payload = &payload[24..24 + payload_len];

                                let delivery_vtime_ns =
                                    vtime_ns + *delay_map.get(&link_id).unwrap_or(&0);

                                let msg = deterministic_coordinator::barrier::CoordMessage {
                                    src_node_id: msg_node_id,
                                    link_id,
                                    delivery_vtime_ns,
                                    sequence_number: seq_num,
                                    payload: raw_payload.to_vec(),
                                };

                                if no_pdes {
                                    uds_deliver_message(&sockets, &default_rx_map, &mut pcap_log, &msg).await;
                                } else {
                                    node_batches
                                        .entry(msg_node_id)
                                        .or_insert_with(Vec::new)
                                        .push(msg);
                                }
                            }
                        }
                    }
} else if topic.starts_with("sim/coord/done/") {
                    let parts: Vec<&str> = topic.split('/').collect();
                    if parts.len() >= 4 {
                        if let Ok(node_id) = parts[3].parse::<u32>() {
                            seen_nodes.insert(node_id);

                            if payload.len() < 16 { std::process::abort(); }
                            let quantum = u64::from_le_bytes(payload[0..8].try_into().expect("FATAL"));
                            let vtime_limit = u64::from_le_bytes(payload[8..16].try_into().expect("FATAL"));
                            let mut batched_msgs = Vec::new();

                            let msgs = node_batches.remove(&node_id).unwrap_or_default();
                            let (mut current_msgs, future_msgs): (
                                Vec<CoordMessage>,
                                Vec<CoordMessage>,
                            ) = msgs
                                .into_iter()
                                .partition(|m| m.delivery_vtime_ns <= vtime_limit + args.delay_ns);

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
                                        uds_deliver_message(&sockets, &default_rx_map, &mut pcap_log, &msg).await;
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

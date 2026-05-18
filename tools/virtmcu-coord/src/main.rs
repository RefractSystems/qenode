#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![deny(unsafe_code)]
use anyhow::{anyhow, bail, Result};
use byteorder::{LittleEndian, WriteBytesExt};
use clap::Parser;
use std::sync::Arc;
use virtmcu_coord::message_log::MessageLog;

use virtmcu_coord::coordinator::{
    CoordinatorAction, CoordinatorConfig, CoordinatorEvent, CoordinatorState, LinkConfig,
};
use virtmcu_coord::topology;

#[derive(Debug)]
struct DummyVTimeProvider;
impl virtmcu_observability::processors::VTimeProvider for DummyVTimeProvider {
    fn current_vtime_ns(&self) -> u64 {
        0
    }
}

fn build_coordinator_config(
    topo: &topology::TopologyGraph,
    n_nodes: usize,
    delay_ns: u64,
    max_messages: usize,
) -> CoordinatorConfig {
    use std::collections::HashMap;
    let mut links = HashMap::new();
    for (link_id, wire_link) in topo.wire_links.iter().enumerate() {
        links.insert(
            wire_link.name.clone(),
            LinkConfig {
                link_id: link_id as u32,
                target_nodes: wire_link.nodes.clone(),
                delay_ns,
            },
        );
    }
    CoordinatorConfig {
        expected_nodes: n_nodes as u32,
        links,
        max_messages_per_node: max_messages,
    }
}

fn build_delivery_frame(
    link_id: u32,
    src_node_id: u32,
    delivery_vtime_ns: u64,
    sequence_number: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(28 + payload.len());
    frame.extend_from_slice(&link_id.to_le_bytes());
    frame.extend_from_slice(&src_node_id.to_le_bytes());
    frame.extend_from_slice(&delivery_vtime_ns.to_le_bytes());
    frame.extend_from_slice(&sequence_number.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
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

    #[arg(short, long)]
    listen: Option<String>,

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

    let transport = if args.transport == "unix" {
        topology::Transport::Unix
    } else {
        topo_raw.transport.clone()
    };

    let topo = Arc::new(tokio::sync::RwLock::new(topo_raw));

    if transport == topology::Transport::Unix {
        run_unix_coordinator(args, federation_id, topo, pcap_log).await
    } else {
        run_virtmcu_coord(args, federation_id, topo, pcap_log).await
    }
}

fn parse_uds_message(
    node_id: u32,
    topic: &str,
    payload: &[u8],
    delay_ns: u64,
) -> Option<CoordinatorEvent> {
    if topic == "sim/coord/link/register" {
        if let Ok(reg) = flatbuffers::root::<virtmcu_wire::LinkRegistration>(payload) {
            return Some(CoordinatorEvent::LinkRegister {
                node_id,
                link_name: reg.link_name().to_string(),
            });
        }
    } else if topic.starts_with("sim/ch/") {
        let parts: Vec<&str> = topic.split('/').collect();
        if parts.len() == 3 {
            if let Ok(link_id) = parts[2].parse::<u32>() {
                if payload.len() >= 24 {
                    let parsed_link_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
                    if parsed_link_id != link_id {
                        std::process::abort();
                    }
                    let payload_len =
                        u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
                    let vtime_ns = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                    let seq_num = u64::from_le_bytes(payload[16..24].try_into().unwrap());

                    if payload.len() >= 24 + payload_len {
                        let raw_payload = payload[24..24 + payload_len].to_vec();
                        let delivery_vtime_ns = vtime_ns.saturating_add(delay_ns);
                        return Some(CoordinatorEvent::PdesMessage {
                            src_node_id: node_id,
                            link_id,
                            delivery_vtime_ns,
                            sequence_number: seq_num,
                            payload: raw_payload,
                        });
                    }
                }
            }
        }
    } else if topic.starts_with("sim/coord/done/") {
        let parts: Vec<&str> = topic.split('/').collect();
        if parts.len() >= 4 {
            if let Ok(msg_node_id) = parts[3].parse::<u32>() {
                if payload.len() >= 16 {
                    let quantum = u64::from_le_bytes(payload[0..8].try_into().unwrap());
                    let vtime_ns = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                    return Some(CoordinatorEvent::QuantumDone {
                        node_id: msg_node_id,
                        quantum,
                        vtime_ns,
                    });
                }
            }
        }
    }
    None
}

async fn execute_uds_actions(
    actions: Vec<CoordinatorAction>,
    sockets: &std::collections::HashMap<
        u32,
        Vec<Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>>,
    >,
    pcap_log: &mut Option<MessageLog>,
) {
    for action in actions {
        match action {
            CoordinatorAction::SendLinkAck { node_id, link_id } => {
                let ack_payload = virtmcu_wire::encode_link_ack(link_id, 0, "");
                if let Some(socks) = sockets.get(&node_id) {
                    for sock in socks {
                        uds_write_framed(sock, "sim/coord/link/ack", &ack_payload).await;
                    }
                }
            }
            CoordinatorAction::BroadcastClockStart { release_quantum } => {
                let payload = virtmcu_wire::encode_uds_quantum_start(release_quantum, u64::MAX);
                let mut sorted_sockets: Vec<_> = sockets.iter().collect();
                sorted_sockets.sort_by_key(|(&id, _)| id);
                for (&node_id, socks) in sorted_sockets {
                    let topic = format!("sim/clock/start/{}", node_id);
                    for sock in socks {
                        uds_write_framed(sock, &topic, &payload).await;
                    }
                }
            }
            CoordinatorAction::RouteMessage {
                target_nodes,
                link_id,
                delivery_vtime_ns,
                sequence_number,
                payload,
            } => {
                let frame =
                    build_delivery_frame(link_id, 0, delivery_vtime_ns, sequence_number, &payload);
                let topic = format!("sim/ch/{}", link_id);
                for target_node in target_nodes {
                    if let Some(socks) = sockets.get(&target_node) {
                        for sock in socks {
                            uds_write_framed(sock, &topic, &frame).await;
                        }
                    }
                }
                if let Some(log) = pcap_log {
                    let msg = virtmcu_coord::barrier::PdesMessage {
                        src_node_id: 0,
                        link_id,
                        delivery_vtime_ns,
                        sequence_number,
                        payload,
                    };
                    let _ = log.write_message(&msg);
                }
            }
            CoordinatorAction::AbortSimulation { reason } => {
                tracing::error!("FATAL: {}", reason);
                std::process::abort();
            }
        }
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

async fn run_unix_coordinator(
    args: Args,
    federation_id: virtmcu_wire::FederationId,
    topo: Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
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

    let delay_ns = args.delay_ns;
    let topo_guard = topo.read().await;
    let coord_config = build_coordinator_config(
        &topo_guard,
        args.nodes,
        args.delay_ns,
        topo_guard.max_messages_per_node_per_quantum,
    );
    drop(topo_guard);
    let mut coord = CoordinatorState::new(coord_config);
    let mut sockets: std::collections::HashMap<
        u32,
        Vec<Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>>,
    > = std::collections::HashMap::new();

    enum WorkerEvent {
        Message(u32, String, Vec<u8>),
        Register(
            u32,
            Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
        ),
        Disconnect(u32),
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
                            std::process::abort();
                        }
                        if reg_fed_id != fed_id_check {
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
                    let _ = worker_tx
                        .send(WorkerEvent::Disconnect(current_node_id))
                        .await;
                });
            }
        }
    });

    let timeout_duration = tokio::time::Duration::from_millis(args.join_timeout_ms);
    let mut joined = false;

    tracing::info!(federation = %federation_id, "Waiting up to {}ms for {} nodes to join...", args.join_timeout_ms, expected_nodes);

    let res = tokio::time::timeout(timeout_duration, async {
        while let Some(evt) = rx_chan.recv().await {
            match evt {
                WorkerEvent::Register(node_id, write_half) => {
                    sockets.entry(node_id).or_default().push(write_half);
                    let actions = coord.apply(CoordinatorEvent::NodeJoined { node_id });
                    for action in &actions {
                        if matches!(action, CoordinatorAction::BroadcastClockStart { .. }) {
                            joined = true;
                        }
                    }
                    execute_uds_actions(actions, &sockets, &mut pcap_log).await;
                }
                WorkerEvent::Disconnect(node_id) => {
                    let actions = coord.apply(CoordinatorEvent::NodeDisconnected { node_id });
                    execute_uds_actions(actions, &sockets, &mut pcap_log).await;
                }
                WorkerEvent::Message(node_id, topic, payload) => {
                    if let Some(event) = parse_uds_message(node_id, &topic, &payload, delay_ns) {
                        let actions = coord.apply(event);
                        for action in &actions {
                            if matches!(action, CoordinatorAction::BroadcastClockStart { .. }) {
                                joined = true;
                            }
                        }
                        execute_uds_actions(actions, &sockets, &mut pcap_log).await;
                    }
                }
            }
            if joined {
                break;
            }
        }
    })
    .await;

    if res.is_err() || !joined {
        bail!(
            "Federation {}: Not all nodes joined within {}ms",
            federation_id.as_str(),
            args.join_timeout_ms
        );
    }

    tracing::info!(federation = %federation_id, "All {} nodes have joined.", expected_nodes);

    while let Some(evt) = rx_chan.recv().await {
        match evt {
            WorkerEvent::Register(node_id, write_half) => {
                sockets.entry(node_id).or_default().push(write_half);
                let actions = coord.apply(CoordinatorEvent::NodeJoined { node_id });
                execute_uds_actions(actions, &sockets, &mut pcap_log).await;
            }
            WorkerEvent::Disconnect(node_id) => {
                let actions = coord.apply(CoordinatorEvent::NodeDisconnected { node_id });
                execute_uds_actions(actions, &sockets, &mut pcap_log).await;
            }
            WorkerEvent::Message(node_id, topic, payload) => {
                if let Some(event) = parse_uds_message(node_id, &topic, &payload, delay_ns) {
                    let actions = coord.apply(event);
                    execute_uds_actions(actions, &sockets, &mut pcap_log).await;
                }
            }
        }
    }

    Ok(())
}

async fn execute_zenoh_actions(
    actions: Vec<CoordinatorAction>,
    session: &zenoh::Session,
    expected_nodes: u32,
    pcap_log: &mut Option<MessageLog>,
) {
    for action in actions {
        match action {
            CoordinatorAction::SendLinkAck { node_id, link_id } => {
                let ack_payload = virtmcu_wire::encode_link_ack(link_id, 0, "");
                let ack_topic = format!("sim/coord/link/ack/{}", node_id);
                let _ = session.put(&ack_topic, ack_payload).await;
            }
            CoordinatorAction::BroadcastClockStart { release_quantum } => {
                for i in 0..expected_nodes {
                    let topic = format!("sim/clock/start/{}", i);
                    let mut payload = Vec::new();
                    payload
                        .write_u64::<LittleEndian>(release_quantum)
                        .expect("Vec write failed");
                    let _ = session.put(&topic, payload).await;
                }
            }
            CoordinatorAction::RouteMessage {
                target_nodes,
                link_id,
                delivery_vtime_ns,
                sequence_number,
                payload,
            } => {
                let frame =
                    build_delivery_frame(link_id, 0, delivery_vtime_ns, sequence_number, &payload);
                for target_node in target_nodes {
                    let rx_topic = format!("sim/ch/{}/{}", link_id, target_node);
                    let _ = session.put(&rx_topic, frame.clone()).await;
                }
                if let Some(log) = pcap_log {
                    let msg = virtmcu_coord::barrier::PdesMessage {
                        src_node_id: 0,
                        link_id,
                        delivery_vtime_ns,
                        sequence_number,
                        payload,
                    };
                    let _ = log.write_message(&msg);
                }
            }
            CoordinatorAction::AbortSimulation { reason } => {
                tracing::error!("FATAL: {}", reason);
                std::process::abort();
            }
        }
    }
}

async fn run_virtmcu_coord(
    args: Args,
    federation_id: virtmcu_wire::FederationId,
    topo: Arc<tokio::sync::RwLock<topology::TopologyGraph>>,
    mut pcap_log: Option<MessageLog>,
) -> Result<()> {
    let mut config = virtmcu_zenoh_config::default_config();
    if let Some(ref l) = args.listen {
        config
            .insert_json5("listen/endpoints", &format!("[\"{}\"]", l))
            .map_err(|e| anyhow!("Invalid Zenoh listen endpoint: {}", e))?;
        config
            .insert_json5("mode", "\"router\"")
            .map_err(|e| anyhow!("Invalid Zenoh mode: {}", e))?;
    } else {
        config
            .insert_json5("mode", "\"client\"")
            .map_err(|e| anyhow!("Invalid Zenoh mode: {}", e))?;
    }

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
        .declare_subscriber("sim/coord/done/*")
        .await
        .map_err(|e| anyhow!("Failed to declare done subscriber: {}", e))?;

    let sub_ctrl = session
        .declare_subscriber("sim/network/control")
        .await
        .map_err(|e| anyhow!("Failed to declare control subscriber: {}", e))?;

    tracing::info!("Coordinator subscribers active");

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

    let mut expected_nodes = std::collections::HashSet::new();
    for i in 0..args.nodes {
        expected_nodes.insert(i as u32);
    }

    let start_time = tokio::time::Instant::now();
    let timeout_duration = tokio::time::Duration::from_millis(args.join_timeout_ms);

    tracing::info!(federation = %federation_id, "Waiting up to {}ms for {} nodes to join...", args.join_timeout_ms, expected_nodes.len());

    let mut joined_nodes = std::collections::HashSet::new();
    let topo_guard = topo.read().await;
    let coord_config = build_coordinator_config(
        &topo_guard,
        args.nodes,
        args.delay_ns,
        topo_guard.max_messages_per_node_per_quantum,
    );
    drop(topo_guard);
    let mut coord = CoordinatorState::new(coord_config);

    while start_time.elapsed() < timeout_duration {
        while let Ok(sample) = rx_chan.try_recv() {
            let topic = sample.key_expr().as_str();
            let payload = sample.payload().to_bytes();

            if topic.starts_with("sim/coord/link/register/") {
                let parts: Vec<&str> = topic.split('/').collect();
                if parts.len() == 5 {
                    if let Ok(msg_node_id) = parts[4].parse::<u32>() {
                        if let Ok(reg) =
                            flatbuffers::root::<virtmcu_wire::LinkRegistration>(&payload)
                        {
                            let link_name = reg.link_name();
                            let actions = coord.apply(CoordinatorEvent::LinkRegister {
                                node_id: msg_node_id,
                                link_name: link_name.to_string(),
                            });
                            execute_zenoh_actions(
                                actions,
                                &session,
                                args.nodes as u32,
                                &mut pcap_log,
                            )
                            .await;
                        }
                    }
                }
            }
        }

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
                        let actions =
                            coord.apply(CoordinatorEvent::NodeJoined { node_id: *node_id });
                        execute_zenoh_actions(actions, &session, args.nodes as u32, &mut pcap_log)
                            .await;
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

    let liveliness_topic = "sim/coord/alive";
    let _liveliness = session
        .liveliness()
        .declare_token(liveliness_topic)
        .await
        .map_err(|e| anyhow!("Failed to declare liveliness token: {}", e))?;

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
                                let actions = coord.apply(CoordinatorEvent::LinkRegister {
                                    node_id: msg_node_id,
                                    link_name: link_name.to_string(),
                                });
                                execute_zenoh_actions(actions, &session, args.nodes as u32, &mut pcap_log).await;
                            }
                        }
                    }
                } else if topic.starts_with("sim/ch/") {
                    let parts: Vec<&str> = topic.split('/').collect();
                    if parts.len() == 4 {
                        if let (Ok(link_id), Ok(msg_node_id)) = (parts[2].parse::<u32>(), parts[3].parse::<u32>()) {
                            if payload.len() >= 24 {
                                let parsed_link_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
                                if parsed_link_id != link_id {
                                    std::process::abort();
                                }
                                let payload_len = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
                                let vtime_ns = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                                let seq_num = u64::from_le_bytes(payload[16..24].try_into().unwrap());

                                if payload.len() >= 24 + payload_len {
                                    let raw_payload = payload[24..24 + payload_len].to_vec();
                                    let delivery_vtime_ns = vtime_ns.saturating_add(args.delay_ns);
                                    let actions = coord.apply(CoordinatorEvent::PdesMessage {
                                        src_node_id: msg_node_id,
                                        link_id,
                                        delivery_vtime_ns,
                                        sequence_number: seq_num,
                                        payload: raw_payload,
                                    });
                                    execute_zenoh_actions(actions, &session, args.nodes as u32, &mut pcap_log).await;
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
                        let payload = sample.payload().to_bytes();
                        if payload.len() >= 16 {
                            let quantum = u64::from_le_bytes(payload[0..8].try_into().unwrap());
                            let vtime_ns = u64::from_le_bytes(payload[8..16].try_into().unwrap());
                            let actions = coord.apply(CoordinatorEvent::QuantumDone {
                                node_id,
                                quantum,
                                vtime_ns,
                            });
                            execute_zenoh_actions(actions, &session, args.nodes as u32, &mut pcap_log).await;
                        } else {
                            std::process::abort();
                        }
                    }
                }
            }
        }
    }
}

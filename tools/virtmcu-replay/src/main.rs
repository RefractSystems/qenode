use anyhow::{anyhow, Result};
use clap::Parser;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};
use virtmcu_wire::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt, ZenohFrameHeader};
use zenoh::query::Query;
use zenoh::Wait as _;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    input: PathBuf,
    #[arg(short, long)]
    topic: String,
    #[arg(short, long, default_value_t = 0)]
    node_id: u32,
    #[arg(short, long)]
    connect: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let mut config = virtmcu_zenoh_config::client_config();
    if let Some(connect) = &args.connect {
        config
            .insert_json5("connect/endpoints", &format!("[\"{}\"]", connect))
            .map_err(|e| anyhow!(format!("{:?}", e)))?;
    }

    info!("Connecting to Zenoh...");
    let session = Arc::new(
        zenoh::open(config)
            .wait()
            .map_err(|e| anyhow!(e.to_string()))?,
    );

    let (query_tx, mut query_rx) = mpsc::channel::<Query>(100);

    let advance_topic = format!("sim/clock/advance/{}", args.node_id);
    let _queryable = session
        .declare_queryable(&advance_topic)
        .callback(move |query| {
            let _ = query_tx.try_send(query);
        })
        .wait()
        .map_err(|e| anyhow!(e.to_string()))?;

    let done_topic = "sim/coord/done";
    let done_pub = session
        .declare_publisher(done_topic)
        .wait()
        .map_err(|e| anyhow!(e.to_string()))?;

    let start_topic = format!("sim/clock/start/{}", args.node_id);
    let start_sub = session
        .declare_subscriber(&start_topic)
        .wait()
        .map_err(|e| anyhow!(e.to_string()))?;

    info!(
        "Replay node started for topic: {} using trace: {}",
        args.topic,
        args.input.display()
    );

    let mcap_file = match File::open(&args.input) {
        Ok(f) => f,
        Err(e) => return Err(anyhow!("Failed to open MCAP file: {}", e)),
    };
    let mmap = unsafe { memmap2::Mmap::map(&mcap_file)? };
    let mut mcap_stream = match mcap::MessageStream::new(&mmap) {
        Ok(s) => s,
        Err(e) => return Err(anyhow!("Failed to read MCAP stream: {}", e)),
    };

    let mut buffered_msg: Option<mcap::Message> = None;
    let mut sequence_number: u64 = 0;

    while let Some(query) = query_rx.recv().await {
        let payload = query.payload().map(|p| p.to_bytes()).unwrap_or_default();
        let req = match ClockAdvanceReq::unpack_slice(&payload) {
            Some(req) => req,
            None => {
                error!("Received malformed ClockAdvanceReq");
                continue;
            }
        };

        let quantum = req.quantum_number();
        let target_vtime = req.absolute_vtime_ns();

        // 1. Signal "done" for this quantum
        let mut done_payload = Vec::with_capacity(16);
        done_payload.extend_from_slice(&quantum.to_le_bytes());
        done_payload.extend_from_slice(&target_vtime.to_le_bytes());
        done_pub
            .put(done_payload)
            .wait()
            .map_err(|e| anyhow!(e.to_string()))?;

        // 2. Wait for coordinator to release the start signal
        let _start_msg = start_sub.recv().map_err(|e| anyhow!(e.to_string()))?;

        // 3. Inject MCAP data for this quantum
        let mut frames_payload = Vec::new();
        let mut n_frames = 0;

        loop {
            let msg = if let Some(m) = buffered_msg.take() {
                m
            } else {
                match mcap_stream.next() {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        error!("MCAP read error: {}", e);
                        break;
                    }
                    None => break, // EOF reached
                }
            };

            let msg_time = msg.log_time;
            let is_target_topic = msg.channel.topic == args.topic;

            if msg_time < target_vtime {
                if is_target_topic {
                    let header =
                        ZenohFrameHeader::new(msg_time, sequence_number, msg.data.len() as u32);
                    frames_payload.extend_from_slice(&header.0);
                    frames_payload.extend_from_slice(&msg.data);
                    n_frames += 1;
                    sequence_number += 1;
                }
            } else {
                buffered_msg = Some(msg);
                break;
            }
        }

        // 4. Respond to query
        let resp = ClockReadyResp::new(target_vtime, n_frames, 0, quantum);
        let mut final_payload = resp.pack().to_vec();
        final_payload.extend_from_slice(&frames_payload); // Append all frames directly after the struct

        query
            .reply(query.key_expr().clone(), final_payload)
            .wait()
            .map_err(|e| anyhow!(e.to_string()))?;
    }

    Ok(())
}

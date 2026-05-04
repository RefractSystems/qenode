use std::env;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <resd_file> <node_id> [delta_ns]", args[0]);
        std::process::exit(1);
    }
    let resd_file = &args[1];
    let node_id: u32 = args[2].parse().unwrap();
    let delta_ns: u64 = if args.len() >= 4 {
        args[3].parse().unwrap()
    } else {
        1_000_000
    };

    let mut parser = cyber_bridge::resd_parser::ResdParser::new(resd_file);
    if !parser.init() {
        eprintln!("[RESD Replay] Failed to parse {resd_file}");
        std::process::exit(1);
    }

    let all_sensors = &parser.sensors;
    let last_ts_ns = parser.get_last_timestamp();

    if all_sensors.is_empty() {
        eprintln!("[RESD Replay] No sensor channels found in {resd_file}");
        std::process::exit(1);
    }

    println!(
        "[RESD Replay] Parsed {} sensor channel(s). Last timestamp: {} ns",
        all_sensors.len(),
        last_ts_ns
    );

    // Zenoh session
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"client\"").unwrap();
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    if let Ok(connect) = env::var("ZENOH_CONNECT") {
        config.insert_json5("connect/endpoints", &connect).unwrap();
    }
    let session = zenoh::open(config).await.unwrap();

    println!("Zenoh session opened successfully.");
    let topic_prefix = env::var("ZENOH_TOPIC_PREFIX").unwrap_or_else(|_| "sim/clock".to_owned());
    let advance_topic = format!("{topic_prefix}/advance/{node_id}");
    println!("[RESD Replay] Node {node_id}: Advance topic: {advance_topic}");
    let mut current_vtime_ns = 0;

    // Simulate stepping until last_ts_ns
    while current_vtime_ns <= last_ts_ns {
        // Send clock advance query
        use virtmcu_api::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt};
        let req = ClockAdvanceReq::new(delta_ns, current_vtime_ns, 0);
        let req_bytes = req.pack();

        let replies = session
            .get(&advance_topic)
            .payload(req_bytes.to_vec())
            .await
            .unwrap();
        let mut got_reply = false;

        while let Ok(reply) = replies.recv_async().await {
            if let Ok(sample) = reply.result() {
                let payload = sample.payload().to_bytes();
                if payload.len() == 24 {
                    let mut arr = [0u8; 24];
                    arr.copy_from_slice(&payload);
                    let resp = ClockReadyResp::unpack_slice(&arr).unwrap();
                    current_vtime_ns = resp.current_vtime_ns();
                    got_reply = true;
                } else {
                    eprintln!(
                        "[RESD Replay] Node {}: Received invalid payload size: {} (expected 24)",
                        node_id,
                        payload.len()
                    );
                }
            }
        }

        if !got_reply {
            eprintln!(
                "[RESD Replay] Node {node_id}: Did not receive ClockReadyPayload for vtime {current_vtime_ns}"
            );
            std::process::exit(1);
        }

        // Publish sensor readings
        for ((sample_type, channel_id), sensor) in all_sensors {
            let topic = format!(
                "sim/sensor/{}/resd_{}_{}",
                node_id, *sample_type as u16, channel_id
            );
            let vals = sensor.get_reading(current_vtime_ns);

            // Format: uint64_t vtime_ns + double values[N]
            let mut payload = Vec::with_capacity(8 + vals.len() * 8);
            payload.extend_from_slice(&current_vtime_ns.to_le_bytes());
            for v in vals {
                payload.extend_from_slice(&v.to_le_bytes());
            }

            let _ = session.put(&topic, payload).await;
        }
    }

    println!("[RESD Replay] Reached end of simulation ({current_vtime_ns} ns). Terminating.");
}

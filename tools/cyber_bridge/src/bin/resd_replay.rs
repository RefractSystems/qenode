use std::env;
use virtmcu_wire::topics::sim_topic;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <resd_file> <node_id> [delta_ns]", args[0]);
        std::process::exit(1);
    }
    let resd_file = &args[1];
    let node_id: u32 = args[2].parse().expect("IO error during setup");
    let delta_ns: u64 = if args.len() >= 4 {
        args[3].parse().expect("IO error during setup")
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
    let mut config = virtmcu_zenoh_config::client_config();
    if let Ok(connect) = env::var("ZENOH_CONNECT") {
        let json_connect = if connect.starts_with('[') && connect.ends_with(']') {
            connect
        } else {
            format!("[\"{connect}\"]")
        };
        config
            .insert_json5("connect/endpoints", &json_connect)
            .expect("IO error during setup");
    }
    let session = zenoh::open(config).await.expect("IO error during setup");

    println!("Zenoh session opened successfully.");
    let node_id_str = node_id.to_string();
    let advance_topic = sim_topic::clock_advance(&node_id_str);
    println!("[RESD Replay] Node {node_id}: Advance topic: {advance_topic}");
    let mut current_vtime_ns = 0;

    // Simulate stepping until last_ts_ns
    while current_vtime_ns <= last_ts_ns {
        // Send clock advance query
        use virtmcu_wire::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt};
        let req = ClockAdvanceReq::new(delta_ns, current_vtime_ns, 0);
        let req_bytes = req.pack();

        let replies = session
            .get(&advance_topic)
            .payload(req_bytes.to_vec())
            .await
            .expect("IO error during setup");
        let mut got_reply = false;

        while let Ok(reply) = replies.recv_async().await {
            if let Ok(sample) = reply.result() {
                let payload = sample.payload().to_bytes();
                if payload.len() == virtmcu_wire::CLOCK_READY_RESP_SIZE {
                    let mut arr = [0u8; virtmcu_wire::CLOCK_READY_RESP_SIZE];
                    arr.copy_from_slice(&payload);
                    let resp = ClockReadyResp::unpack_slice(&arr).expect("IO error during setup");
                    current_vtime_ns = resp.current_vtime_ns();
                    got_reply = true;
                } else {
                    eprintln!(
                        "[RESD Replay] Node {}: Received invalid payload size: {} (expected {})",
                        node_id,
                        payload.len(),
                        virtmcu_wire::CLOCK_READY_RESP_SIZE
                    );
                }
            }
        }

        if !got_reply {
            eprintln!(
                "[RESD Replay] Node {node_id}: Did not receive ClockReadyResp for vtime {current_vtime_ns}"
            );
            std::process::exit(1);
        }

        // Publish sensor readings
        for ((_sample_type, channel_id), sensor) in all_sensors {
            let topic = format!("sim/sensor/{}/sensordata_{}", node_id, channel_id);
            let vals = sensor.get_reading(current_vtime_ns);

            let mut data_payload = Vec::with_capacity(vals.len() * 8);
            for v in vals {
                data_payload.extend_from_slice(&v.to_le_bytes());
            }

            let payload = virtmcu_wire::encode_frame(current_vtime_ns, 0, &data_payload);
            println!(
                "[RESD Replay] Publishing sensor data to {} at vtime {}",
                topic, current_vtime_ns
            );
            let _ = session.put(&topic, payload).await;
        }
    }

    println!("[RESD Replay] Reached end of simulation ({current_vtime_ns} ns). Terminating.");
}

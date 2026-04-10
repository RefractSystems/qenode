use std::collections::HashSet;
use clap::Parser;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Cursor, Write};
use zenoh::config::Config;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Propagation delay to add to the virtual timestamp (in nanoseconds)
    #[arg(short, long, default_value_t = 1_000_000)]
    delay_ns: u64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    println!("Starting virtmcu Zenoh Coordinator (delay: {} ns)", args.delay_ns);

    let session = zenoh::open(Config::default()).await.unwrap();

    // Subscribe to all TX topics
    let subscriber = session.declare_subscriber("sim/eth/frame/*/tx").await.unwrap();

    // Track active nodes dynamically based on who transmits
    let mut known_nodes = HashSet::new();

    println!("Listening for packets on sim/eth/frame/*/tx...");

    while let Ok(sample) = subscriber.recv_async().await {
        let topic = sample.key_expr().as_str();
        let parts: Vec<&str> = topic.split('/').collect();
        if parts.len() != 5 {
            continue;
        }
        let sender_id = parts[3].to_string();
        known_nodes.insert(sender_id.clone());

        let payload = sample.payload().to_bytes();
        if payload.len() < 12 {
            continue;
        }

        let mut cursor = Cursor::new(&payload);
        let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
        let size = cursor.read_u32::<LittleEndian>().unwrap();

        let new_delivery_vtime_ns = delivery_vtime_ns + args.delay_ns;

        let mut new_payload = Vec::with_capacity(payload.len());
        new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns).unwrap();
        new_payload.write_u32::<LittleEndian>(size).unwrap();
        new_payload.write_all(&payload[12..]).unwrap();

        // Broadcast to all known nodes except the sender
        for node in &known_nodes {
            if node != &sender_id {
                let rx_topic = format!("sim/eth/frame/{}/rx", node);
                if let Err(e) = session.put(&rx_topic, new_payload.clone()).await {
                    eprintln!("Failed to forward to {}: {}", node, e);
                } else {
                    println!(
                        "Forwarded {} bytes from Node {} to Node {} (vtime: {} -> {})",
                        size, sender_id, node, delivery_vtime_ns, new_delivery_vtime_ns
                    );
                }
            }
        }
    }
}

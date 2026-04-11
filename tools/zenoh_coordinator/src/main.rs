/*
 * virtmcu Zenoh Coordinator
 *
 * This Rust daemon replaces the concept of a traditional "WirelessMedium" or
 * central network switch found in other emulation frameworks (like Renode).
 *
 * In a deterministic multi-node simulation, nodes cannot communicate over
 * standard UDP or TCP sockets because the host OS network stack introduces
 * non-deterministic latency. Instead, all inter-node communication (Ethernet,
 * UART, SystemC CAN) flows through Zenoh.
 *
 * The Coordinator's role:
 * 1. Topology Discovery: It dynamically discovers nodes when they publish to
 *    TX topics (e.g., `sim/eth/frame/node0/tx`).
 * 2. Causal Ordering: It reads the `delivery_vtime_ns` timestamp from the
 *    incoming message's header, adds a configurable propagation `delay_ns`,
 *    and rewrites the timestamp.
 * 3. Broadcast: It republishes the message to the RX topics of all *other*
 *    known nodes in the network (e.g., `sim/eth/frame/node1/rx`).
 *
 * Because the receiving nodes use `hw/zenoh/zenoh-netdev.c` (or equivalent),
 * they will buffer the message and deliver it into the guest firmware *only*
 * when their virtual clocks catch up to the rewritten delivery timestamp.
 */
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
    let eth_sub = session.declare_subscriber("sim/eth/frame/*/tx").await.unwrap();
    let uart_sub = session.declare_subscriber("virtmcu/uart/*/tx").await.unwrap();
    let sysc_sub = session.declare_subscriber("sim/systemc/frame/*/tx").await.unwrap();

    // Track active nodes dynamically based on who transmits
    let mut known_eth_nodes = HashSet::new();
    let mut known_uart_nodes = HashSet::new();
    let mut known_sysc_nodes = HashSet::new();

    println!("Listening for packets on sim/eth/frame/*/tx, virtmcu/uart/*/tx, and sim/systemc/frame/*/tx...");

    loop {
        tokio::select! {
            Ok(sample) = eth_sub.recv_async() => {
                handle_eth_msg(&session, sample, &mut known_eth_nodes, args.delay_ns).await;
            }
            Ok(sample) = uart_sub.recv_async() => {
                handle_uart_msg(&session, sample, &mut known_uart_nodes, args.delay_ns).await;
            }
            Ok(sample) = sysc_sub.recv_async() => {
                handle_sysc_msg(&session, sample, &mut known_sysc_nodes, args.delay_ns).await;
            }
        }
    }
}

async fn handle_eth_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    delay_ns: u64,
) {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 5 {
        return;
    }
    let sender_id = parts[3].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return;
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
    let size = cursor.read_u32::<LittleEndian>().unwrap();

    let new_delivery_vtime_ns = delivery_vtime_ns + delay_ns;

    let mut new_payload = Vec::with_capacity(payload.len());
    new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns).unwrap();
    new_payload.write_u32::<LittleEndian>(size).unwrap();
    new_payload.write_all(&payload[12..]).unwrap();

    // Broadcast to all known nodes except the sender
    for node in known_nodes.iter() {
        if node != &sender_id {
            let rx_topic = format!("sim/eth/frame/{}/rx", node);
            if let Err(e) = session.put(&rx_topic, new_payload.clone()).await {
                eprintln!("Failed to forward to {}: {}", node, e);
            } else {
                println!(
                    "ETH: Forwarded {} bytes from {} to {} (vtime: {} -> {})",
                    size, sender_id, node, delivery_vtime_ns, new_delivery_vtime_ns
                );
            }
        }
    }
}

async fn handle_uart_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    delay_ns: u64,
) {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 4 {
        return;
    }
    let sender_id = parts[2].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return;
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
    let size = cursor.read_u32::<LittleEndian>().unwrap();

    let new_delivery_vtime_ns = delivery_vtime_ns + delay_ns;

    let mut new_payload = Vec::with_capacity(payload.len());
    new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns).unwrap();
    new_payload.write_u32::<LittleEndian>(size).unwrap();
    new_payload.write_all(&payload[12..]).unwrap();

    // Broadcast to all known nodes except the sender
    for node in known_nodes.iter() {
        if node != &sender_id {
            let rx_topic = format!("virtmcu/uart/{}/rx", node);
            if let Err(e) = session.put(&rx_topic, new_payload.clone()).await {
                eprintln!("Failed to forward to {}: {}", node, e);
            } else {
                println!(
                    "UART: Forwarded {} bytes from {} to {} (vtime: {} -> {})",
                    size, sender_id, node, delivery_vtime_ns, new_delivery_vtime_ns
                );
            }
        }
    }
}

async fn handle_sysc_msg(
    session: &zenoh::Session,
    sample: zenoh::sample::Sample,
    known_nodes: &mut HashSet<String>,
    delay_ns: u64,
) {
    let topic = sample.key_expr().as_str();
    let parts: Vec<&str> = topic.split('/').collect();
    if parts.len() != 5 {
        return;
    }
    let sender_id = parts[3].to_string();
    known_nodes.insert(sender_id.clone());

    let payload = sample.payload().to_bytes();
    if payload.len() < 12 {
        return;
    }

    let mut cursor = Cursor::new(&payload);
    let delivery_vtime_ns = cursor.read_u64::<LittleEndian>().unwrap();
    let size = cursor.read_u32::<LittleEndian>().unwrap();

    let new_delivery_vtime_ns = delivery_vtime_ns + delay_ns;

    let mut new_payload = Vec::with_capacity(payload.len());
    new_payload.write_u64::<LittleEndian>(new_delivery_vtime_ns).unwrap();
    new_payload.write_u32::<LittleEndian>(size).unwrap();
    new_payload.write_all(&payload[12..]).unwrap();

    // Broadcast to all known nodes except the sender
    for node in known_nodes.iter() {
        if node != &sender_id {
            let rx_topic = format!("sim/systemc/frame/{}/rx", node);
            if let Err(e) = session.put(&rx_topic, new_payload.clone()).await {
                eprintln!("Failed to forward to {}: {}", node, e);
            } else {
                println!(
                    "SYSC: Forwarded {} bytes from {} to {} (vtime: {} -> {})",
                    size, sender_id, node, delivery_vtime_ns, new_delivery_vtime_ns
                );
            }
        }
    }
}

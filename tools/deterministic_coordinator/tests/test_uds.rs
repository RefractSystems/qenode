use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::sleep;
use transport_unix::UdsDataTransport;
use virtmcu_api::{decode_coord_message, encode_coord_done_req, DataTransport};

#[tokio::test]
async fn test_uds_coordinator_pdes() {
    let federation_id = "test_uds_pdes";
    let run_dir = "/tmp/virtmcu_test_run";
    let sock_path = format!("{}/{}/coordinator.sock", run_dir, federation_id);
    let _ = std::fs::remove_file(&sock_path);

    let topo_yaml = r#"
max_messages_per_node_per_quantum: 100
topology:
  transport: unix
  nodes:
    - name: '0'
    - name: '1'
  links:
    - type: Ethernet
      nodes: ['0', '1']
"#;
    let topo_path = "/tmp/test_uds_topo.yaml";
    std::fs::write(topo_path, topo_yaml).unwrap();

    let mut coord = Command::new(env!("CARGO_BIN_EXE_deterministic_coordinator"))
        .env("VIRTMCU_RUN_DIR", run_dir)
        .arg("--federation-id")
        .arg(federation_id)
        .arg("--nodes")
        .arg("2")
        .arg("--topology")
        .arg(topo_path)
        .arg("--join-timeout-ms")
        .arg("5000")
        // use --delay-ns 0 so vtime remains exactly what we set
        .arg("--delay-ns")
        .arg("0")
        .spawn()
        .expect("Failed to start coordinator");

    sleep(Duration::from_millis(500)).await;

    let t0 = Arc::new(
        UdsDataTransport::new_with_fed_id(&sock_path, 0, federation_id)
            .expect("Failed to connect node 0"),
    );
    let t1 = Arc::new(
        UdsDataTransport::new_with_fed_id(&sock_path, 1, federation_id)
            .expect("Failed to connect node 1"),
    );

    let rx0 = Arc::new(Mutex::new(Vec::new()));
    let rx0_clone = rx0.clone();
    t0.subscribe(
        "sim/coord/0/rx",
        Box::new(move |_: &str, payload: &[u8]| {
            rx0_clone.lock().unwrap().push(payload.to_vec());
        }),
    )
    .unwrap();

    let rx1 = Arc::new(Mutex::new(Vec::new()));
    let rx1_clone = rx1.clone();
    t1.subscribe(
        "sim/coord/1/rx",
        Box::new(move |_: &str, payload: &[u8]| {
            rx1_clone.lock().unwrap().push(payload.to_vec());
        }),
    )
    .unwrap();

    // Give it a moment to process registrations
    sleep(Duration::from_millis(500)).await;

    // Send TX messages. We use sim/chardev/0/tx which parses ZenohFrameHeader (24 bytes).
    let payload0 = virtmcu_api::encode_frame(10, 2, b"hello from 0");
    t0.publish("sim/chardev/0/tx", &payload0).unwrap();

    let payload1 = virtmcu_api::encode_frame(5, 1, b"hello from 1");
    t1.publish("sim/chardev/1/tx", &payload1).unwrap();

    // Now send DONE messages for quantum 0
    let done_payload = encode_coord_done_req(0, 100); // quantum 0, vtime_limit 100

    t0.publish_raw("sim/coord/done/0/q/0", &done_payload)
        .unwrap();
    t1.publish_raw("sim/coord/done/1/q/0", &done_payload)
        .unwrap();

    // Wait for delivery
    sleep(Duration::from_millis(1000)).await;

    let received0 = rx0.lock().unwrap().clone();
    let received1 = rx1.lock().unwrap().clone();

    assert_eq!(received0.len(), 1, "Node 0 should receive 1 message");
    assert_eq!(received1.len(), 1, "Node 1 should receive 1 message");

    let (vtime0, seq0, _data0) = decode_coord_message(&received0[0]).unwrap();
    assert_eq!(vtime0, 5);
    assert_eq!(seq0, 1);

    let (vtime1, seq1, _data1) = decode_coord_message(&received1[0]).unwrap();
    assert_eq!(vtime1, 10);
    assert_eq!(seq1, 2);

    coord.kill().await.unwrap();
}

#[tokio::test]
async fn test_uds_multi_socket() {
    let federation_id = "test_uds_multi";
    let run_dir = "/tmp/virtmcu_test_run_multi";
    let sock_path = format!("{}/{}/coordinator.sock", run_dir, federation_id);
    let _ = std::fs::remove_file(&sock_path);

    let topo_yaml = r#"
topology:
  transport: unix
  nodes:
    - name: '0'
"#;
    let topo_path = "/tmp/test_uds_topo_multi.yaml";
    std::fs::write(topo_path, topo_yaml).unwrap();

    let mut coord = Command::new(env!("CARGO_BIN_EXE_deterministic_coordinator"))
        .env("VIRTMCU_RUN_DIR", run_dir)
        .arg("--federation-id")
        .arg(federation_id)
        .arg("--nodes")
        .arg("1")
        .arg("--topology")
        .arg(topo_path)
        .spawn()
        .expect("Failed to start coordinator");

    sleep(Duration::from_millis(500)).await;

    // Two connections for the same node
    let t0_a = Arc::new(
        UdsDataTransport::new_with_fed_id(&sock_path, 0, federation_id)
            .expect("Failed to connect socket A"),
    );
    let t0_b = Arc::new(
        UdsDataTransport::new_with_fed_id(&sock_path, 0, federation_id)
            .expect("Failed to connect socket B"),
    );

    let rx_a = Arc::new(Mutex::new(0));
    let rx_a_clone = rx_a.clone();
    t0_a.subscribe(
        "sim/clock/start/0",
        Box::new(move |_, _| {
            *rx_a_clone.lock().unwrap() += 1;
        }),
    )
    .unwrap();

    let rx_b = Arc::new(Mutex::new(0));
    let rx_b_clone = rx_b.clone();
    t0_b.subscribe(
        "sim/clock/start/0",
        Box::new(move |_, _| {
            *rx_b_clone.lock().unwrap() += 1;
        }),
    )
    .unwrap();

    sleep(Duration::from_millis(500)).await;

    // Send DONE
    let done_payload = encode_coord_done_req(0, 100);
    t0_a.publish_raw("sim/coord/done/0/q/0", &done_payload)
        .unwrap();

    sleep(Duration::from_millis(500)).await;

    // Both sockets should have received the START signal
    assert_eq!(*rx_a.lock().unwrap(), 1, "Socket A should receive START");
    assert_eq!(*rx_b.lock().unwrap(), 1, "Socket B should receive START");

    coord.kill().await.unwrap();
}

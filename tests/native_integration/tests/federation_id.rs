#[tokio::test]
async fn test_federation_id_propagates_to_logs() {
    let env = virtmcu_test_runner::TestContext::new().unwrap();
    let socket_path = env.tmp_path("virtmcu-test.sock");
    let socket_path_str = socket_path.to_str().unwrap();

    let fed_id = "test-federation-001";
    // Using a mock or real binary if available in the environment.
    // Assuming virtmcu-physical-node is in the path or target/debug.
    let bin_path = if std::path::Path::new("../../target/release/virtmcu-physical-node").exists() {
        "../../target/release/virtmcu-physical-node"
    } else if std::path::Path::new("../../target/debug/virtmcu-physical-node").exists() {
        "../../target/debug/virtmcu-physical-node"
    } else if std::path::Path::new("./target/release/virtmcu-physical-node").exists() {
        "./target/release/virtmcu-physical-node"
    } else {
        "virtmcu-physical-node"
    };

    let mut pn = tokio::process::Command::new(bin_path)
        .env("RUST_LOG", "info")
        .arg("--federation-id")
        .arg(fed_id)
        .arg("--node-id")
        .arg("0")
        .arg("--plant")
        .arg("tick-only")
        .arg("--transport")
        .arg("unix")
        .arg("--connect")
        .arg(socket_path_str)
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn virtmcu-physical-node");

    // Wait a bit for it to start and emit some logs.
    // Since we don't have the mock transport listener yet, it might warn about transport.
    // virtmcu-allow: test_sleep reasoning="wait for node to start"
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let _ = pn.kill().await;

    let output = pn.wait_with_output().await.unwrap();
    let logs = String::from_utf8_lossy(&output.stdout);
    let logs_err = String::from_utf8_lossy(&output.stderr);
    let all_logs = format!("{}{}", logs, logs_err);

    assert!(
        all_logs.contains(fed_id),
        "Expected federation ID '{}' in logs: \nSTDOUT: {}\nSTDERR: {}",
        fed_id,
        logs,
        logs_err
    );
}

#[tokio::test]
async fn test_federation_id_required() {
    let bin_path = if std::path::Path::new("../../target/release/virtmcu-physical-node").exists() {
        "../../target/release/virtmcu-physical-node"
    } else if std::path::Path::new("../../target/debug/virtmcu-physical-node").exists() {
        "../../target/debug/virtmcu-physical-node"
    } else if std::path::Path::new("./target/release/virtmcu-physical-node").exists() {
        "./target/release/virtmcu-physical-node"
    } else {
        "virtmcu-physical-node"
    };

    let output = tokio::process::Command::new(bin_path)
        .arg("--node-id")
        .arg("0") // deliberately omit --federation-id
        .output()
        .await
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "Expected failure when federation-id is missing, but it succeeded.\nSTDOUT: {}\nSTDERR: {}",
        stdout,
        stderr
    );
    assert!(
        stderr.contains("federation-id") || stderr.contains("VIRTMCU_FEDERATION_ID"),
        "Error message must mention the missing flag: {}",
        stderr
    );
}

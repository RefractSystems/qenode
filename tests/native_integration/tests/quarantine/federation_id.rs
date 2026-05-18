use tokio::io::AsyncBufReadExt;

#[tokio::test]
async fn test_federation_id_propagates_to_logs() {
    let env = virtmcu_test_runner::TestContext::new().unwrap();
    let socket_path = env.tmp_path("virtmcu-test.sock");
    let socket_path_str = socket_path.to_str().unwrap();

    let fed_id = "test-federation-001";
    let bin_path = env.find_binary("virtmcu-physical-node").unwrap();

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

    let stdout = pn.stdout.take().unwrap();
    let stderr = pn.stderr.take().unwrap();

    let mut reader_stdout = tokio::io::BufReader::new(stdout).lines();
    let mut reader_stderr = tokio::io::BufReader::new(stderr).lines();

    let mut all_logs = String::new();
    let mut found = false;
    let mut stdout_done = false;
    let mut stderr_done = false;

    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !stdout_done || !stderr_done {
            tokio::select! {
                res = reader_stdout.next_line(), if !stdout_done => {
                    match res {
                        Ok(Some(line)) => {
                            all_logs.push_str(&line);
                            all_logs.push('\n');
                            if line.contains(fed_id) {
                                found = true;
                                break;
                            }
                        }
                        _ => stdout_done = true,
                    }
                }
                res = reader_stderr.next_line(), if !stderr_done => {
                    match res {
                        Ok(Some(line)) => {
                            all_logs.push_str(&line);
                            all_logs.push('\n');
                            if line.contains(fed_id) {
                                found = true;
                                break;
                            }
                        }
                        _ => stderr_done = true,
                    }
                }
            }
        }
    })
    .await;

    let _ = pn.kill().await;

    assert!(
        found,
        "Expected federation ID '{}' in logs: \nLOGS: {}",
        fed_id, all_logs
    );
}

#[tokio::test]
async fn test_federation_id_required() {
    let env = virtmcu_test_runner::TestContext::new().unwrap();
    let bin_path = env.find_binary("virtmcu-physical-node").unwrap();

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

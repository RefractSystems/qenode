use anyhow::Result;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use virtmcu_test_runner::{NodeConfig, TestContext, VirtmcuTestEnv};
use virtmcu_wire::{
    FlatBufferStructExt, VirtmcuHandshake, VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_svd_hash_handshake_rejection() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let ctx = TestContext::new().unwrap();
    let socket_path = ctx.tmp_path("virtmcu_test_svd_hash.sock");
    let yaml_path = ctx.tmp_path("test_svd_hash_board.yaml");

    let listener = UnixListener::bind(&socket_path)?;

    let yaml = format!(
        r#"
machine:
  cpus:
    - name: cpu0
      type: cortex-m4
peripherals:
  - name: bridge0
    type: mmio-socket-bridge
    address: none
    properties:
      region-size: 0x1000
      socket-path: {}
      svd: hw/defs/actuator.svd
"#,
        socket_path.to_string_lossy()
    );

    std::fs::write(&yaml_path, yaml)?;

    // Compute the expected correct hash
    let svd_content = std::fs::read_to_string("hw/defs/actuator.svd")?;
    use std::hash::{Hash, Hasher};
    let mut hasher = fnv::FnvHasher::default();
    svd_content.hash(&mut hasher);
    let correct_svd_hash = hasher.finish();

    // Simulate a client with the WRONG hash
    let bad_hash = correct_svd_hash ^ 0xDEADBEEF;

    let test_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        let mut buf = [0u8; virtmcu_wire::VIRTMCU_HANDSHAKE_SIZE];
        stream.read_exact(&mut buf).await.unwrap();
        let client_hs = VirtmcuHandshake::unpack_slice(&buf).unwrap();

        assert_eq!(client_hs.magic(), VIRTMCU_PROTO_MAGIC);
        assert_eq!(client_hs.version(), VIRTMCU_PROTO_VERSION);
        assert_eq!(client_hs.svd_hash(), correct_svd_hash);

        // Send handshake with a BAD hash
        let our_hs = VirtmcuHandshake::new(VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION, bad_hash);
        stream.write_all(our_hs.pack()).await.unwrap();

        // The emulator should reject it and drop the connection
        let mut dummy = [0u8; 1];
        let res = stream.read(&mut dummy).await.unwrap();
        assert_eq!(res, 0); // 0 means EOF (disconnected)
    });

    // Run emulator
    let _env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/actuator/actuator.elf")
                .with_yaml_path(yaml_path.to_str().unwrap())
                .orchestrated(false),
        )
        .with_timeout(5)
        .build()
        .await?;

    tokio::time::timeout(Duration::from_secs(5), test_task).await??;

    Ok(())
}

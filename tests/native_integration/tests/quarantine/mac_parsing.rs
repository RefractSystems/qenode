use anyhow::Result;
use virtmcu_test_runner::{NodeConfig, VirtmcuTestEnv};

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_macaddr_parsing() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    let yaml_content = r#"
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals:
  - name: ram
    type: Memory.MappedMemory
    address: 0x40000000
    properties:
      size: 0x1000000
  - name: test_dev
    type: test-rust-device
    address: sysbus
    properties:
      MACAddress: "00:11:22:33:44:55"
"#;

    let tmp_yaml = std::env::temp_dir().join("test_mac.yml");
    std::fs::write(&tmp_yaml, yaml_content)?;

    let mut env = VirtmcuTestEnv::builder()
        .add_node(
            NodeConfig::new(0)
                .with_firmware_path("tests/fixtures/guest_apps/boot_arm/hello.elf")
                .with_yaml_path(tmp_yaml.to_str().unwrap())
                .orchestrated(false)
                .add_qemu_arg("-S"),
        )
        .with_timeout(10)
        .build()
        .await?;

    let qmp = env.qmp(0);

    // We can just query the device directly if we know where it is, or use qom-list.
    // Let's use qom-list to find it like the python test.
    let mut queue = vec!["/machine".to_string()];
    let mut dev_path = None;

    while let Some(path) = queue.pop() {
        if let Ok(res) = qmp
            .execute_with_args("qom-list", Some(serde_json::json!({"path": path})))
            .await
        {
            if let Some(returns) = res["return"].as_array() {
                for obj in returns {
                    let name = obj["name"].as_str().unwrap();
                    if name == "type" || name == "child<qemu:memory-region>" || name == "parent_bus"
                    {
                        continue;
                    }
                    let child_path = if path == "/" {
                        format!("/{}", name)
                    } else {
                        format!("{}/{}", path, name)
                    };

                    if let Ok(t_res) = qmp
                        .execute_with_args(
                            "qom-get",
                            Some(serde_json::json!({"path": child_path, "property": "type"})),
                        )
                        .await
                    {
                        if t_res["return"].as_str() == Some("test-rust-device") {
                            dev_path = Some(child_path);
                            break;
                        }
                    }
                    if child_path.matches('/').count() < 5 {
                        queue.push(child_path);
                    }
                }
            }
        }
        if dev_path.is_some() {
            break;
        }
    }

    assert!(dev_path.is_some(), "test-rust-device not found");

    let mac_res = qmp
        .execute_with_args(
            "qom-get",
            Some(serde_json::json!({"path": dev_path.unwrap(), "property": "macaddr"})),
        )
        .await?;
    assert_eq!(mac_res["return"].as_str(), Some("00:11:22:33:44:55"));

    Ok(())
}

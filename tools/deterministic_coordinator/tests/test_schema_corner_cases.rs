// This file tests edge cases in deserialization using the generated Typify models.
use deterministic_coordinator::generated::topology::WorldSchema;

#[test]
fn test_reject_invalid_protocol() {
    let yaml = r#"
topology:
  links:
    - type: "invalid_protocol"
      nodes: ["0", "1"]
"#;
    let result: Result<WorldSchema, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err());
}

#[test]
fn test_address_hex_string_validation() {
    let valid_yaml = r#"
peripherals:
  - name: "valid_mem"
    address: "0x1A2B3C"
    size: 4096
"#;
    let world: WorldSchema = serde_yaml::from_str(valid_yaml).expect("Valid YAML should parse");
    let peripherals = world.peripherals;
    let addr = peripherals[0].address.clone().unwrap();
    match addr {
        deterministic_coordinator::generated::topology::Address::String(s) => {
            assert_eq!(s.as_str(), "0x1A2B3C");
        }
        _ => panic!("Expected Address::String"),
    }

    let invalid_yaml = r#"
peripherals:
  - name: "invalid_mem"
    address: "1A2B3C" # Missing 0x prefix
"#;
    let result: Result<WorldSchema, _> = serde_yaml::from_str(invalid_yaml);
    assert!(
        result.is_err(),
        "Invalid hex string should fail deserialization"
    );
}

#[test]
fn test_legacy_nodes_block() {
    let yaml = r#"
nodes:
  - name: "0"
  - name: "1"
"#;
    let world: WorldSchema = serde_yaml::from_str(yaml).unwrap();
    let nodes = world.nodes;
    assert_eq!(nodes.len(), 2);
    match &nodes[0].name {
        deterministic_coordinator::generated::topology::NodeId::String(s) => {
            assert_eq!(s.as_str(), "0")
        }
        _ => panic!("Expected NodeId::String"),
    }
}

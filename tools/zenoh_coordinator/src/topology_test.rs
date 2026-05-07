use crate::topology::*;
use std::path::Path;

#[test]
fn test_yaml_number_coercion() {
    let yaml_str = r#"
topology:
  global_seed: 42
  links:
    - type: uart
      nodes: [0, 1]
    "#;
    let world: YamlWorld = serde_yaml::from_str(yaml_str).expect("test should succeed");
    let topo = world.topology.expect("test should succeed");
    assert_eq!(topo.links[0].nodes[0], "0");
}

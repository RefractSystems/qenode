use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ethernet,
    Uart,
    Spi,
    CanFd,
    FlexRay,
    Lin,
    Rf802154,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Zenoh,
    Unix,
}

// Convert from string to u32
fn string_to_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse::<u32>().map_err(serde::de::Error::custom)
}

fn vec_string_to_u32<'de, D>(deserializer: D) -> Result<Vec<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let strings: Vec<String> = Deserialize::deserialize(deserializer)?;
    let mut out = Vec::new();
    for s in strings {
        out.push(s.parse::<u32>().map_err(serde::de::Error::custom)?);
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireLink {
    #[serde(rename = "type")]
    pub protocol: Protocol,
    #[serde(deserialize_with = "vec_string_to_u32")]
    pub nodes: Vec<u32>,
    pub baud: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WirelessNode {
    #[serde(deserialize_with = "string_to_u32")]
    pub id: u32,
    pub initial_position: [f64; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WirelessMedium {
    pub medium: String,
    pub nodes: Vec<WirelessNode>,
    pub max_range_m: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyConfig {
    #[serde(default = "default_max_messages")]
    pub max_messages_per_node_per_quantum: usize,
    #[serde(default)]
    pub global_seed: u64,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub links: Vec<WireLink>,
    pub wireless: Option<WirelessMedium>,
}

fn default_max_messages() -> usize {
    1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlNode {
    #[serde(deserialize_with = "string_to_u32")]
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlWorld {
    #[serde(default)]
    pub nodes: Vec<YamlNode>,
    pub topology: Option<TopologyConfig>,
}

#[derive(Debug)]
pub enum TopologyError {
    IoError(std::io::Error),
    YamlError(serde_yaml::Error),
    UnknownNode(u32),
}

impl std::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopologyError::IoError(e) => write!(f, "IO Error: {}", e),
            TopologyError::YamlError(e) => write!(f, "YAML Error: {}", e),
            TopologyError::UnknownNode(n) => write!(f, "Unknown node in topology: {}", n),
        }
    }
}

impl std::error::Error for TopologyError {}

impl From<std::io::Error> for TopologyError {
    fn from(err: std::io::Error) -> Self {
        TopologyError::IoError(err)
    }
}

impl From<serde_yaml::Error> for TopologyError {
    fn from(err: serde_yaml::Error) -> Self {
        TopologyError::YamlError(err)
    }
}

#[derive(Debug)]
pub struct TopologyGraph {
    pub max_messages_per_node_per_quantum: usize,
    pub global_seed: u64,
    pub transport: Transport,
    wire_links: Vec<WireLink>,
    wireless_medium: Option<WirelessMedium>,
    node_positions: HashMap<u32, [f64; 3]>,
    max_wireless_range_m: f64,
    pub is_explicit: bool,
}

impl TopologyGraph {
    pub fn from_yaml(path: &Path) -> Result<Self, TopologyError> {
        let content = fs::read_to_string(path)?;
        let world: YamlWorld = serde_yaml::from_str(&content)?;

        let mut valid_nodes = HashSet::new();
        for node in world.nodes {
            valid_nodes.insert(node.id);
        }

        if let Some(topo) = world.topology {
            let mut positions = HashMap::new();
            let mut max_range = 0.0;

            for link in &topo.links {
                for node_id in &link.nodes {
                    if !valid_nodes.contains(node_id) {
                        return Err(TopologyError::UnknownNode(*node_id));
                    }
                }
            }

            if let Some(ref wl) = topo.wireless {
                max_range = wl.max_range_m;
                for n in &wl.nodes {
                    if !valid_nodes.contains(&n.id) {
                        return Err(TopologyError::UnknownNode(n.id));
                    }
                    positions.insert(n.id, n.initial_position);
                }
            }

            Ok(TopologyGraph {
                max_messages_per_node_per_quantum: topo.max_messages_per_node_per_quantum,
                global_seed: topo.global_seed,
                transport: topo.transport,
                wire_links: topo.links,
                wireless_medium: topo.wireless,
                node_positions: positions,
                max_wireless_range_m: max_range,
                is_explicit: true,
            })
        } else {
            Ok(TopologyGraph::default())
        }
    }
}

impl Default for TopologyGraph {
    fn default() -> Self {
        TopologyGraph {
            max_messages_per_node_per_quantum: 1024,
            global_seed: 0,
            transport: Transport::default(),
            wire_links: Vec::new(),
            wireless_medium: None,
            node_positions: HashMap::new(),
            max_wireless_range_m: 0.0,
            is_explicit: false,
        }
    }
}

impl TopologyGraph {
    pub fn has_wireless(&self) -> bool {
        self.wireless_medium.is_some()
    }

    pub fn is_link_allowed(&self, src: u32, dst: u32, protocol: Protocol) -> bool {
        if !self.is_explicit {
            return true; // implicitly allow all if no topology is loaded
        }
        for link in &self.wire_links {
            if link.protocol == protocol && link.nodes.contains(&src) && link.nodes.contains(&dst) {
                return true;
            }
        }
        false
    }

    pub fn update_positions(&mut self, updates: &[(u32, [f64; 3])]) {
        for (id, pos) in updates {
            self.node_positions.insert(*id, *pos);
        }
    }

    pub fn rf_neighbors(&self, node_id: u32) -> Vec<u32> {
        let mut neighbors = Vec::new();
        if self.wireless_medium.is_none() {
            return neighbors;
        }

        if let Some(my_pos) = self.node_positions.get(&node_id) {
            for (other_id, other_pos) in &self.node_positions {
                if *other_id == node_id {
                    continue;
                }
                let dx = my_pos[0] - other_pos[0];
                let dy = my_pos[1] - other_pos[1];
                let dz = my_pos[2] - other_pos[2];
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                // We add a tiny epsilon (1e-6) to handle floating point inaccuracies
                if dist <= self.max_wireless_range_m + 1e-6 {
                    neighbors.push(*other_id);
                }
            }
        }
        neighbors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wire_link_bidirectional() {
        let yaml_str = r#"
nodes:
  - id: 0
  - id: 1
topology:
  links:
    - type: ethernet
      nodes: [0, 1]
        "#;
        let path = std::env::temp_dir().join(format!(
            "test_topo_{}.yaml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        fs::write(&path, yaml_str).unwrap();

        let tg = TopologyGraph::from_yaml(&path).unwrap();
        assert!(tg.is_link_allowed(0, 1, Protocol::Ethernet));
        assert!(tg.is_link_allowed(1, 0, Protocol::Ethernet));
    }

    #[test]
    fn test_wire_link_no_cross_protocol() {
        let yaml_str = r#"
nodes:
  - id: 0
  - id: 1
topology:
  links:
    - type: ethernet
      nodes: [0, 1]
        "#;
        let path = std::env::temp_dir().join(format!(
            "test_topo_{}.yaml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        fs::write(&path, yaml_str).unwrap();

        let tg = TopologyGraph::from_yaml(&path).unwrap();
        assert!(!tg.is_link_allowed(0, 1, Protocol::Uart));
    }

    #[test]
    fn test_wireless_in_range() {
        let mut tg = TopologyGraph {
            transport: Transport::Zenoh,
            max_messages_per_node_per_quantum: 1024,
            global_seed: 0,
            wire_links: vec![],
            wireless_medium: Some(WirelessMedium {
                medium: "ieee802154".to_string(),
                nodes: vec![],
                max_range_m: 10.0,
            }),
            node_positions: HashMap::new(),
            max_wireless_range_m: 10.0,
            is_explicit: true,
        };

        tg.update_positions(&[(0, [0.0, 0.0, 0.0]), (1, [5.0, 0.0, 0.0])]);

        let n0 = tg.rf_neighbors(0);
        assert!(n0.contains(&1));
    }

    #[test]
    fn test_wireless_out_of_range() {
        let mut tg = TopologyGraph {
            transport: Transport::Zenoh,
            max_messages_per_node_per_quantum: 1024,
            global_seed: 0,
            wire_links: vec![],
            wireless_medium: Some(WirelessMedium {
                medium: "ieee802154".to_string(),
                nodes: vec![],
                max_range_m: 10.0,
            }),
            node_positions: HashMap::new(),
            max_wireless_range_m: 10.0,
            is_explicit: true,
        };

        tg.update_positions(&[(0, [0.0, 0.0, 0.0]), (1, [15.0, 0.0, 0.0])]);

        let n0 = tg.rf_neighbors(0);
        assert!(!n0.contains(&1));
    }

    #[test]
    fn test_position_update_changes_neighbors() {
        let mut tg = TopologyGraph {
            transport: Transport::Zenoh,
            max_messages_per_node_per_quantum: 1024,
            global_seed: 0,
            wire_links: vec![],
            wireless_medium: Some(WirelessMedium {
                medium: "ieee802154".to_string(),
                nodes: vec![],
                max_range_m: 10.0,
            }),
            node_positions: HashMap::new(),
            max_wireless_range_m: 10.0,
            is_explicit: true,
        };

        // start node 1 at [15,0,0]
        tg.update_positions(&[(0, [0.0, 0.0, 0.0]), (1, [15.0, 0.0, 0.0])]);

        assert!(!tg.rf_neighbors(0).contains(&1));

        // call update_positions
        tg.update_positions(&[(1, [5.0, 0.0, 0.0])]);

        assert!(tg.rf_neighbors(0).contains(&1));
    }

    #[test]
    fn test_topology_unknown_node_rejected() {
        let yaml_str = r#"
nodes:
  - id: 0
topology:
  links:
    - type: uart
      nodes: [0, 99]
        "#;
        let path = std::env::temp_dir().join(format!(
            "test_topo_{}.yaml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        fs::write(&path, yaml_str).unwrap();

        let result = TopologyGraph::from_yaml(&path);
        match result {
            Err(TopologyError::UnknownNode(99)) => (), // Success
            _ => panic!("Expected UnknownNode(99), got {:?}", result),
        }
    }
}

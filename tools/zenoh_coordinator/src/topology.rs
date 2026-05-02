use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ethernet,
    Uart,
    Spi,
    CanFd,
    FlexRay,
    Lin,
    Rf802154,
    RfHci,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireLink {
    #[serde(rename = "type")]
    pub protocol: Protocol,
    pub nodes: Vec<String>,
    pub baud: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WirelessNode {
    pub id: String,
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
    #[serde(default)]
    pub global_seed: Option<u64>,
    #[serde(default)]
    pub links: Vec<WireLink>,
    pub wireless: Option<WirelessMedium>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlWorld {
    pub topology: Option<TopologyConfig>,
}

#[derive(Debug)]
pub enum TopologyError {
    IoError(std::io::Error),
    YamlError(serde_yaml::Error),
}

impl core::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TopologyError::IoError(e) => write!(f, "IO Error: {e}"),
            TopologyError::YamlError(e) => write!(f, "YAML Error: {e}"),
        }
    }
}

impl core::error::Error for TopologyError {}

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

pub struct TopologyGraph {
    pub global_seed: Option<u64>,
    wire_links: Vec<WireLink>,
    wireless_medium: Option<WirelessMedium>,
    node_positions: HashMap<String, [f64; 3]>,
    max_wireless_range_m: f64,
    pub is_explicit: bool,
    pub max_messages_per_node_per_quantum: usize,
}

impl Default for TopologyGraph {
    fn default() -> Self {
        TopologyGraph {
            global_seed: None,
            wire_links: Vec::new(),
            wireless_medium: None,
            node_positions: HashMap::new(),
            max_wireless_range_m: 0.0,
            is_explicit: false,
            max_messages_per_node_per_quantum: 1024,
        }
    }
}

impl TopologyGraph {
    pub fn from_yaml(path: &Path) -> Result<Self, TopologyError> {
        let content = fs::read_to_string(path)?;
        let world: YamlWorld = serde_yaml::from_str(&content)?;

        if let Some(topo) = world.topology {
            let mut positions = HashMap::new();
            let mut max_range = 0.0;

            if let Some(ref wl) = topo.wireless {
                max_range = wl.max_range_m;
                for n in &wl.nodes {
                    positions.insert(n.id.clone(), n.initial_position);
                }
            }

            Ok(TopologyGraph {
                global_seed: topo.global_seed,
                wire_links: topo.links,
                wireless_medium: topo.wireless,
                node_positions: positions,
                max_wireless_range_m: max_range,
                is_explicit: true,
                max_messages_per_node_per_quantum: 1024,
            })
        } else {
            // Default allow-all topology if no topology section is defined
            Ok(TopologyGraph::default())
        }
    }

    pub fn has_wireless(&self) -> bool {
        self.wireless_medium.is_some()
    }

    pub fn is_link_allowed(
        &self,
        src_node_id: &str,
        dst_node_id: &str,
        protocol: &Protocol,
    ) -> bool {
        if !self.is_explicit {
            return true;
        }

        for link in &self.wire_links {
            if &link.protocol == protocol
                && link.nodes.iter().any(|n| n == src_node_id)
                && link.nodes.iter().any(|n| n == dst_node_id)
            {
                return true;
            }
        }

        false
    }

    pub fn get_wire_peers(&self, node_id: &str, protocol: &Protocol) -> HashSet<String> {
        let mut peers = HashSet::new();
        if !self.is_explicit {
            return peers;
        }

        for link in &self.wire_links {
            if &link.protocol == protocol && link.nodes.iter().any(|n| n == node_id) {
                for node in &link.nodes {
                    if node != node_id {
                        peers.insert(node.clone());
                    }
                }
            }
        }

        peers
    }

    pub fn update_positions(&mut self, updates: Vec<(String, [f64; 3])>) {
        for (id, pos) in updates {
            self.node_positions.insert(id, pos);
        }
    }

    pub fn rf_neighbors(&self, node_id: &str) -> HashSet<String> {
        let mut neighbors = HashSet::new();
        if self.wireless_medium.is_none() {
            return neighbors;
        }

        if let Some(my_pos) = self.node_positions.get(node_id) {
            for (other_id, other_pos) in &self.node_positions {
                if other_id == node_id {
                    continue;
                }
                let dx = my_pos[0] - other_pos[0];
                let dy = my_pos[1] - other_pos[1];
                let dz = my_pos[2] - other_pos[2];
                let dist = (dx * dx + dy * dy + dz * dz).sqrt();
                // We add a tiny epsilon (1e-6) to handle floating point inaccuracies
                if dist <= self.max_wireless_range_m + 1e-6 {
                    neighbors.insert(other_id.clone());
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
    fn test_yaml_number_coercion() {
        let yaml_str = r#"
topology:
  global_seed: 42
  links:
    - type: uart
      nodes: [0, 1]
    - type: ethernet
      nodes: ["2", "3"]
        "#;
        let world: YamlWorld = serde_yaml::from_str(yaml_str).unwrap();
        let topo = world.topology.unwrap();
        assert_eq!(topo.global_seed, Some(42));
        assert_eq!(topo.links.len(), 2);
        assert_eq!(topo.links[0].nodes, vec!["0".to_owned(), "1".to_owned()]);
        assert_eq!(topo.links[1].nodes, vec!["2".to_owned(), "3".to_owned()]);
    }

    #[test]
    fn test_is_link_allowed() {
        let tg = TopologyGraph {
            global_seed: None,
            wire_links: vec![WireLink {
                protocol: Protocol::Uart,
                nodes: vec!["0".to_owned(), "1".to_owned()],
                baud: None,
            }],
            wireless_medium: None,
            node_positions: HashMap::new(),
            max_wireless_range_m: 0.0,
            is_explicit: true,
            max_messages_per_node_per_quantum: 1024,
        };

        assert!(tg.is_link_allowed("0", "1", &Protocol::Uart));
        assert!(tg.is_link_allowed("1", "0", &Protocol::Uart));

        // Not allowed because protocol mismatch
        assert!(!tg.is_link_allowed("0", "1", &Protocol::Ethernet));
        // Not allowed because node missing
        assert!(!tg.is_link_allowed("0", "2", &Protocol::Uart));
    }

    #[test]
    fn test_is_link_allowed_implicit_allow_all() {
        let tg = TopologyGraph::default();
        assert!(tg.is_link_allowed("0", "1", &Protocol::Uart));
        assert!(tg.is_link_allowed("10", "20", &Protocol::Ethernet));
    }

    #[test]
    fn test_rf_neighbors() {
        let mut tg = TopologyGraph {
            global_seed: None,
            wire_links: vec![],
            wireless_medium: Some(WirelessMedium {
                medium: "ieee802154".to_owned(),
                nodes: vec![],
                max_range_m: 10.0,
            }),
            node_positions: HashMap::new(),
            max_wireless_range_m: 10.0,
            is_explicit: true,
            max_messages_per_node_per_quantum: 1024,
        };

        tg.update_positions(vec![
            ("0".to_owned(), [0.0, 0.0, 0.0]),
            ("1".to_owned(), [5.0, 0.0, 0.0]),  // Dist 5
            ("2".to_owned(), [10.0, 0.0, 0.0]), // Dist 10 (on boundary)
            ("3".to_owned(), [11.0, 0.0, 0.0]), // Dist 11 (out of range)
        ]);

        let n0 = tg.rf_neighbors("0");
        assert!(n0.contains("1"));
        assert!(n0.contains("2"));
        assert!(!n0.contains("3"));
        assert!(!n0.contains("0")); // Should not contain itself

        let n2 = tg.rf_neighbors("2");
        assert!(n2.contains("0"));
        assert!(n2.contains("1"));
        assert!(n2.contains("3"));
    }
}

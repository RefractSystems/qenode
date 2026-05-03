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
    Control,
}

impl Protocol {
    pub fn is_wireless(&self) -> bool {
        matches!(self, Protocol::Rf802154 | Protocol::RfHci)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Zenoh,
    Unix,
}

// Convert from string or number to u32, gracefully skipping non-integers
fn string_to_u32_or_max<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrU32 {
        String(String),
        U32(u32),
    }

    match StringOrU32::deserialize(deserializer)? {
        StringOrU32::String(s) => {
            match s.parse::<u32>() {
                Ok(u) => Ok(u),
                Err(_) => Ok(u32::MAX), // Marker for non-integer names (like 'memory')
            }
        }
        StringOrU32::U32(u) => Ok(u),
    }
}

fn vec_string_to_u32<'de, D>(deserializer: D) -> Result<Vec<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrU32 {
        String(String),
        U32(u32),
    }

    let values: Vec<StringOrU32> = Deserialize::deserialize(deserializer)?;
    let mut out = Vec::new();
    for v in values {
        match v {
            StringOrU32::String(s) => {
                if let Ok(u) = s.parse::<u32>() {
                    out.push(u);
                }
            }
            StringOrU32::U32(u) => out.push(u),
        }
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
    #[serde(rename = "name", deserialize_with = "string_to_u32_or_max")]
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
    #[serde(default)]
    pub nodes: Option<Vec<YamlNode>>,
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
    #[serde(rename = "name", deserialize_with = "string_to_u32_or_max")]
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlWorld {
    #[serde(rename = "peripherals", default)]
    pub legacy_peripherals: serde_yaml::Value,
    pub topology: Option<TopologyConfig>,
}

#[derive(Debug)]
pub enum TopologyError {
    IoError(std::io::Error),
    YamlError(serde_yaml::Error),
    UnknownNode(u32),
    SplitBrainError(String),
}

impl core::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TopologyError::IoError(e) => write!(f, "IO Error: {e}"),
            TopologyError::YamlError(e) => write!(f, "YAML Error: {e}"),
            TopologyError::UnknownNode(n) => write!(f, "Unknown node in topology: {n}"),
            TopologyError::SplitBrainError(s) => write!(f, "Split-brain Error: {s}"),
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
    drop_list: HashSet<(u32, u32)>,
}

impl TopologyGraph {
    pub fn from_yaml(path: &Path) -> Result<Self, TopologyError> {
        let content = fs::read_to_string(path)?;
        let world: YamlWorld = serde_yaml::from_str(&content)?;

        // Task 2.2: Split-Brain Schema Rejection
        let has_topology_nodes = world
            .topology
            .as_ref()
            .and_then(|t| t.nodes.as_ref())
            .is_some_and(|n| !n.is_empty());

        let mut has_numeric_periphs = false;
        if let Some(nodes_list) = world.legacy_peripherals.as_sequence() {
            has_numeric_periphs = nodes_list.iter().any(|item| {
                item.get("name")
                    .and_then(|n| match n {
                        serde_yaml::Value::String(s) => Some(s.clone()),
                        serde_yaml::Value::Number(num) => Some(num.to_string()),
                        _ => None,
                    })
                    .is_some_and(|s| s.parse::<u32>().is_ok())
            });
        }

        if has_topology_nodes && has_numeric_periphs {
            return Err(TopologyError::SplitBrainError(
                "Split-brain YAML detected: both 'topology.nodes' and numeric 'peripherals' are present.".to_owned(),
            ));
        }

        let mut valid_nodes = HashSet::new();

        // 1. Try to get nodes from topology.nodes
        if let Some(ref topo) = world.topology {
            if let Some(ref nodes) = topo.nodes {
                for node in nodes {
                    if node.id != u32::MAX {
                        valid_nodes.insert(node.id);
                    }
                }
            }
        }

        // 2. Fallback to legacy top-level peripherals if topology.nodes is missing
        if valid_nodes.is_empty() && !world.legacy_peripherals.is_null() {
            if let Some(nodes_list) = world.legacy_peripherals.as_sequence() {
                let mut fallback_nodes = Vec::new();
                let mut all_numeric = true;

                for item in nodes_list {
                    if let Some(name) = item.get("name") {
                        let name_str = match name {
                            serde_yaml::Value::String(s) => Some(s.clone()),
                            serde_yaml::Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        };

                        if let Some(s) = name_str {
                            if let Ok(id) = s.parse::<u32>() {
                                fallback_nodes.push(id);
                            } else {
                                all_numeric = false;
                                break;
                            }
                        } else {
                            all_numeric = false;
                            break;
                        }
                    } else {
                        all_numeric = false;
                        break;
                    }
                }

                if all_numeric && !fallback_nodes.is_empty() {
                    tracing::warn!("DEPRECATION: Top-level 'peripherals' for topology nodes is deprecated. Move them to 'topology.nodes'.");
                    for id in fallback_nodes {
                        valid_nodes.insert(id);
                    }
                }
            }
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
                    if n.id != u32::MAX {
                        if !valid_nodes.contains(&n.id) {
                            return Err(TopologyError::UnknownNode(n.id));
                        }
                        positions.insert(n.id, n.initial_position);
                    }
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
                drop_list: HashSet::new(),
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
            drop_list: HashSet::new(),
        }
    }
}

impl TopologyGraph {
    pub fn has_wireless(&self) -> bool {
        self.wireless_medium.is_some()
    }

    pub fn wire_links(&self) -> &[WireLink] {
        &self.wire_links
    }

    pub fn is_link_allowed(&self, src: u32, dst: u32, protocol: Protocol) -> bool {
        if self.drop_list.contains(&(src, dst)) || self.drop_list.contains(&(dst, src)) {
            return false;
        }
        if !self.is_explicit {
            return true; // implicitly allow all if no topology is loaded
        }
        if protocol.is_wireless() {
            return self.rf_neighbors(src).contains(&dst);
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
                if dist <= self.max_wireless_range_m + 1e-6 {
                    neighbors.push(*other_id);
                }
            }
        }
        neighbors
    }

    pub fn update_from_json(&mut self, json_str: &str) -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::Value;
        let v: Value = serde_json::from_str(json_str)?;

        if let (Some(from_val), Some(to_val), Some(drop_prob)) = (
            v.get("from").and_then(|f| f.as_str()),
            v.get("to").and_then(|t| t.as_str()),
            v.get("drop_probability").and_then(|d| d.as_f64()),
        ) {
            let from_node = from_val.parse::<u32>()?;
            let to_node = to_val.parse::<u32>()?;

            if drop_prob >= 0.99 {
                self.drop_list.insert((from_node, to_node));
            } else if drop_prob <= 0.01 {
                self.drop_list.remove(&(from_node, to_node));
                self.drop_list.remove(&(to_node, from_node));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_topology_nodes_new_schema() {
        let content = r#"
topology:
  nodes:
    - name: "1"
    - name: 2
  links:
    - type: uart
      nodes: [1, 2]
"#;
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{}", content).unwrap();
        let graph = TopologyGraph::from_yaml(file.path()).unwrap();
        assert!(graph.is_link_allowed(1, 2, Protocol::Uart));
    }

    #[test]
    fn test_topology_fallback_legacy_schema() {
        let content = r#"
peripherals:
  - name: "1"
  - name: 2
topology:
  links:
    - type: uart
      nodes: [1, 2]
"#;
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{}", content).unwrap();
        let graph = TopologyGraph::from_yaml(file.path()).unwrap();
        assert!(graph.is_link_allowed(1, 2, Protocol::Uart));
    }

    #[test]
    fn test_topology_no_fallback_machine_schema() {
        let content = r#"
peripherals:
  - name: uart0
    renode_type: UART.PL011
topology:
  nodes:
    - name: "1"
  links: []
"#;
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{}", content).unwrap();
        let graph = TopologyGraph::from_yaml(file.path()).unwrap();
        // Should not fail parsing 'uart0' because it's not falling back
        assert!(graph.is_explicit);
    }

    #[test]
    fn test_schema_split_brain_rejection() {
        let content = r#"
peripherals:
  - name: "1"
topology:
  nodes:
    - name: "2"
  links: []
"#;
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{}", content).unwrap();
        let res = TopologyGraph::from_yaml(file.path());
        assert!(res.is_err());
        assert!(format!("{}", res.unwrap_err()).contains("Split-brain"));
    }

    #[test]
    fn test_schema_legacy_fallback_with_warning() {
        let content = r#"
peripherals:
  - name: "1"
"#;
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{}", content).unwrap();
        let graph = TopologyGraph::from_yaml(file.path()).unwrap();
        assert!(!graph.is_explicit);
    }
}

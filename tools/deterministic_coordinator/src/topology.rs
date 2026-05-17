#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
use crate::generated::topology as gen;
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
    #[serde(rename = "reference-link")]
    ReferenceLink,
}

impl Protocol {
    pub fn is_wireless(&self) -> bool {
        matches!(self, Protocol::Rf802154 | Protocol::RfHci)
    }
}

impl From<gen::Protocol> for Protocol {
    fn from(p: gen::Protocol) -> Self {
        match p.to_string().to_lowercase().as_str() {
            "ethernet" | "eth" => Protocol::Ethernet,
            "uart" => Protocol::Uart,
            "spi" => Protocol::Spi,
            "canfd" => Protocol::CanFd,
            "flexray" => Protocol::FlexRay,
            "lin" => Protocol::Lin,
            "rf802154" => Protocol::Rf802154,
            "rfhci" => Protocol::RfHci,
            "reference-link" => Protocol::ReferenceLink,
            _ => Protocol::Control,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Zenoh,
    Unix,
}

impl From<gen::TopologyTransport> for Transport {
    fn from(t: gen::TopologyTransport) -> Self {
        match t {
            gen::TopologyTransport::Zenoh => Transport::Zenoh,
            gen::TopologyTransport::Unix => Transport::Unix,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireLink {
    #[serde(rename = "type")]
    pub protocol: Protocol,
    pub nodes: Vec<u32>,
    pub baud: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WirelessNode {
    pub id: u32,
    pub initial_position: [f64; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WirelessMedium {
    pub medium: String,
    pub nodes: Vec<WirelessNode>,
    pub max_range_m: f64,
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
    pub routing_map: RoutingMap,
}

fn node_id_to_u32(nid: &gen::NodeId) -> u32 {
    match nid {
        gen::NodeId::Integer(i) => *i as u32,
        gen::NodeId::String(s) => s.parse::<u32>().expect("Invalid data format"),
    }
}

impl TopologyGraph {
    pub fn from_yaml(path: &Path) -> Result<Self, TopologyError> {
        let content = fs::read_to_string(path)?;

        let mut world_topology: Option<gen::Topology> = None;
        let mut world_peripherals: Vec<gen::Resource> = Vec::new();

        // First try to parse as strict gen::World
        if let Ok(world) = serde_yaml::from_str::<gen::World>(&content) {
            world_topology = world.topology;
            world_peripherals = world.peripherals;
        } else {
            // Fallback: manually extract 'topology' to tolerate multi-node schemas
            let value: serde_yaml::Value = serde_yaml::from_str(&content)?;
            if let Some(topo_val) = value.get("topology") {
                if let Ok(topo) = serde_yaml::from_value(topo_val.clone()) {
                    world_topology = Some(topo);
                }
            }
            if let Some(serde_yaml::Value::Sequence(seq)) = value.get("peripherals") {
                for item in seq {
                    if let Ok(res) = serde_yaml::from_value(item.clone()) {
                        world_peripherals.push(res);
                    }
                }
            }
        }

        // Task 2.2: Split-Brain Schema Rejection
        let has_topology_nodes = world_topology.as_ref().is_some_and(|t| !t.nodes.is_empty());

        let has_numeric_periphs = world_peripherals.iter().any(|p| match &p.name {
            gen::NodeId::Integer(_) => true,
            gen::NodeId::String(s) => s.parse::<u32>().is_ok(),
        });

        if has_topology_nodes && has_numeric_periphs {
            return Err(TopologyError::SplitBrainError(
                "Split-brain YAML detected: both 'topology.nodes' and numeric 'peripherals' are present.".to_owned(),
            ));
        }

        let mut valid_nodes = HashSet::new();

        // 1. Try to get nodes from topology.nodes
        if let Some(ref topo) = world_topology {
            for node in &topo.nodes {
                let id = node_id_to_u32(&node.name);
                if id != u32::MAX {
                    valid_nodes.insert(id);
                }
            }
        }

        // 2. Fallback to legacy top-level peripherals if topology.nodes is missing
        if valid_nodes.is_empty() {
            let mut fallback_nodes = Vec::new();
            let mut all_numeric = true;

            for p in &world_peripherals {
                let id = node_id_to_u32(&p.name);
                if id != u32::MAX {
                    fallback_nodes.push(id);
                } else {
                    all_numeric = false;
                    break;
                }
            }

            if all_numeric && !fallback_nodes.is_empty() {
                tracing::debug!("DEPRECATION: Top-level 'peripherals' for topology nodes is deprecated. Move them to 'topology.nodes'.");
                for id in fallback_nodes {
                    valid_nodes.insert(id);
                }
            }
        }

        if let Some(topo) = world_topology {
            let mut routing_map = RoutingMap::new();
            let mut positions = HashMap::new();
            let mut max_range = 0.0;

            let mut wire_links = Vec::new();
            for link in &topo.links {
                let mut nodes = Vec::new();
                for nid in &link.nodes {
                    let id = node_id_to_u32(nid);
                    if !valid_nodes.contains(&id) {
                        return Err(TopologyError::UnknownNode(id));
                    }
                    nodes.push(id);
                    for i in 0..nodes.len() {
                        for j in 0..nodes.len() {
                            if i != j {
                                routing_map.add_route(nodes[i].to_string(), nodes[j].to_string());
                            }
                        }
                    }
                }
                wire_links.push(WireLink {
                    protocol: Protocol::from(link.type_.clone()),
                    nodes,
                    baud: link.baud,
                });
            }

            let mut wireless_medium = None;
            if let Some(wl) = &topo.wireless {
                max_range = wl.max_range_m;
                let mut nodes = Vec::new();
                for n in &wl.nodes {
                    let id = node_id_to_u32(&n.name);
                    if id != u32::MAX {
                        if !valid_nodes.contains(&id) {
                            return Err(TopologyError::UnknownNode(id));
                        }
                        positions.insert(
                            id,
                            [
                                n.initial_position.x,
                                n.initial_position.y,
                                n.initial_position.z,
                            ],
                        );
                        nodes.push(WirelessNode {
                            id,
                            initial_position: [
                                n.initial_position.x,
                                n.initial_position.y,
                                n.initial_position.z,
                            ],
                        });
                    }
                }
                wireless_medium = Some(WirelessMedium {
                    medium: wl.medium.clone(),
                    nodes,
                    max_range_m: wl.max_range_m,
                });
            }

            Ok(TopologyGraph {
                max_messages_per_node_per_quantum: topo
                    .max_messages_per_node_per_quantum
                    .unwrap_or(1024) as usize,
                global_seed: topo
                    .global_seed
                    .as_ref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0), // virtmcu-allow: unwrap_or_fallback reasoning="global_seed is optional and defaults to 0"
                transport: topo.transport.map(Transport::from).unwrap_or_default(),
                wire_links,
                wireless_medium,
                node_positions: positions,
                max_wireless_range_m: max_range,
                is_explicit: true,
                drop_list: HashSet::new(),
                routing_map,
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
            routing_map: RoutingMap::new(),
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

#[derive(Debug, Clone, Default)]
pub struct RoutingMap {
    pub map: std::collections::HashMap<String, Vec<String>>,
}

impl RoutingMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_route(&mut self, source: String, target: String) {
        self.map.entry(source).or_default().push(target);
    }

    pub fn get_targets(&self, source: &str) -> Option<&Vec<String>> {
        self.map.get(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_routing_map_basic() {
        let mut map = RoutingMap::new();
        map.add_route("A".to_string(), "B".to_string());
        assert_eq!(map.get_targets("A").unwrap(), &vec!["B".to_string()]);
        assert!(map.get_targets("X").is_none());
    }

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
        let mut file = NamedTempFile::new().expect("test should succeed");
        writeln!(file, "{}", content).expect("test should succeed");
        let graph = TopologyGraph::from_yaml(file.path()).expect("test should succeed");
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
        let mut file = NamedTempFile::new().expect("test should succeed");
        writeln!(file, "{}", content).expect("test should succeed");
        let graph = TopologyGraph::from_yaml(file.path()).expect("test should succeed");
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
        let mut file = NamedTempFile::new().expect("test should succeed");
        writeln!(file, "{}", content).expect("test should succeed");
        let graph = TopologyGraph::from_yaml(file.path()).expect("test should succeed");
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
        let mut file = NamedTempFile::new().expect("test should succeed");
        writeln!(file, "{}", content).expect("test should succeed");
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
        let mut file = NamedTempFile::new().expect("test should succeed");
        writeln!(file, "{}", content).expect("test should succeed");
        let graph = TopologyGraph::from_yaml(file.path()).expect("test should succeed");
        assert!(!graph.is_explicit);
    }

    #[test]
    fn test_ping_pong_parse() {
        let content = std::fs::read_to_string("../../worlds/reference_ping_pong.yml").unwrap();
        let mut file = NamedTempFile::new().expect("test should succeed");
        writeln!(file, "{}", content).expect("test should succeed");
        let graph = TopologyGraph::from_yaml(file.path()).expect("test should succeed");
        println!("MAP IS: {:?}", graph.routing_map.map);
    }
}

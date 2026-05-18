#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkConfig {
    pub link_id: u32,
    pub target_nodes: Vec<u32>,
    pub delay_ns: u64,
}

#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    pub expected_nodes: u32,
    pub links: HashMap<String, LinkConfig>,
    pub max_messages_per_node: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorEvent {
    NodeJoined {
        node_id: u32,
    },
    NodeDisconnected {
        node_id: u32,
    },
    LinkRegister {
        node_id: u32,
        link_name: String,
    },
    QuantumDone {
        node_id: u32,
        quantum: u64,
        vtime_ns: u64,
    },
    SimulationMessage {
        src_node_id: u32,
        link_id: u32,
        delivery_vtime_ns: u64,
        sequence_number: u64,
        payload: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorAction {
    SendLinkAck {
        node_id: u32,
        link_id: u32,
    },
    BroadcastClockStart {
        release_quantum: u64,
    },
    RouteMessage {
        target_nodes: Vec<u32>,
        link_id: u32,
        delivery_vtime_ns: u64,
        sequence_number: u64,
        payload: Vec<u8>,
    },
    AbortSimulation {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingMessage {
    src_node_id: u32,
    link_id: u32,
    delivery_vtime_ns: u64,
    sequence_number: u64,
    payload: Vec<u8>,
}

impl PartialOrd for PendingMessage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PendingMessage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.delivery_vtime_ns
            .cmp(&other.delivery_vtime_ns)
            .then(self.src_node_id.cmp(&other.src_node_id))
            .then(self.sequence_number.cmp(&other.sequence_number))
    }
}

#[derive(Debug, Clone)]
struct BarrierState {
    n_nodes: usize,
    max_messages_per_node: usize,
    current_quantum: u64,
    done_nodes: Vec<bool>,
    message_buffer: Vec<PendingMessage>,
    next_quantum_done_nodes: Vec<bool>,
    next_quantum_buffer: Vec<PendingMessage>,
}

#[derive(Debug)]
enum BarrierError {
    NodeIndexOutOfBounds,
    QuantumMismatch,
    DuplicateDone,
}

impl BarrierState {
    fn new(n_nodes: usize, max_messages_per_node: usize) -> Self {
        Self {
            n_nodes,
            max_messages_per_node,
            current_quantum: 0,
            done_nodes: vec![false; n_nodes],
            message_buffer: Vec::new(),
            next_quantum_done_nodes: vec![false; n_nodes],
            next_quantum_buffer: Vec::new(),
        }
    }

    fn submit_done(
        &mut self,
        node_id: u32,
        quantum: u64,
        mut messages: Vec<PendingMessage>,
    ) -> Result<Option<Vec<PendingMessage>>, BarrierError> {
        let node_idx = node_id as usize;
        if node_idx >= self.n_nodes {
            return Err(BarrierError::NodeIndexOutOfBounds);
        }

        if quantum < self.current_quantum {
            return Ok(None);
        }

        if quantum > self.current_quantum + 1 {
            return Err(BarrierError::QuantumMismatch);
        }

        if quantum == self.current_quantum + 1 {
            if self.next_quantum_done_nodes[node_idx] {
                return Err(BarrierError::DuplicateDone);
            }
            self.next_quantum_done_nodes[node_idx] = true;
            self.next_quantum_buffer.append(&mut messages);

            if self.next_quantum_done_nodes.iter().all(|&d| d) {
                tracing::error!(
                    "ALL {} nodes submitted quantum={} as LOOKAHEAD",
                    self.n_nodes,
                    quantum
                );
            }
            return Ok(None);
        }

        if self.done_nodes[node_idx] {
            return Err(BarrierError::DuplicateDone);
        }

        self.done_nodes[node_idx] = true;
        messages.sort_unstable();
        messages.truncate(self.max_messages_per_node);
        self.message_buffer.append(&mut messages);

        if self.done_nodes.iter().all(|&d| d) {
            let mut all_msgs = std::mem::take(&mut self.message_buffer);
            all_msgs.sort_unstable();

            self.current_quantum += 1;
            self.done_nodes = std::mem::take(&mut self.next_quantum_done_nodes);
            self.next_quantum_done_nodes = vec![false; self.n_nodes];
            self.message_buffer = std::mem::take(&mut self.next_quantum_buffer);

            Ok(Some(all_msgs))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug)]
enum Phase {
    AwaitingNodes {
        joined_nodes: HashSet<u32>,
        buffered_link_regs: Vec<(u32, String)>,
    },
    AwaitingLinks {
        remaining: HashSet<(u32, String)>,
    },
    Simulation {
        node_batches: HashMap<u32, Vec<PendingMessage>>,
        barrier: BarrierState,
    },
}

pub struct CoordinatorState {
    config: CoordinatorConfig,
    phase: Phase,
}

impl CoordinatorState {
    pub fn new(config: CoordinatorConfig) -> Self {
        Self {
            config,
            phase: Phase::AwaitingNodes {
                joined_nodes: HashSet::new(),
                buffered_link_regs: Vec::new(),
            },
        }
    }

    pub fn apply(&mut self, event: CoordinatorEvent) -> Vec<CoordinatorAction> {
        match &mut self.phase {
            Phase::AwaitingNodes {
                joined_nodes,
                buffered_link_regs,
            } => match event {
                CoordinatorEvent::NodeJoined { node_id } => {
                    joined_nodes.insert(node_id);
                    if joined_nodes.len() as u32 == self.config.expected_nodes {
                        let mut remaining = HashSet::new();
                        for (link_name, link_cfg) in &self.config.links {
                            for &target_node in &link_cfg.target_nodes {
                                remaining.insert((target_node, link_name.clone()));
                            }
                        }

                        let mut actions = Vec::new();
                        let buffered = std::mem::take(buffered_link_regs);

                        self.phase = Phase::AwaitingLinks {
                            remaining: remaining.clone(),
                        };

                        if remaining.is_empty() {
                            let mut node_batches = HashMap::new();
                            for i in 0..self.config.expected_nodes {
                                node_batches.insert(i, Vec::new());
                            }
                            self.phase = Phase::Simulation {
                                node_batches,
                                barrier: BarrierState::new(
                                    self.config.expected_nodes as usize,
                                    self.config.max_messages_per_node,
                                ),
                            };
                            actions.push(CoordinatorAction::BroadcastClockStart {
                                release_quantum: 0,
                            });
                        } else {
                            for (b_node, b_link) in buffered {
                                actions.extend(self.apply(CoordinatorEvent::LinkRegister {
                                    node_id: b_node,
                                    link_name: b_link,
                                }));
                            }
                        }

                        actions
                    } else {
                        vec![]
                    }
                }
                CoordinatorEvent::LinkRegister { node_id, link_name } => {
                    buffered_link_regs.push((node_id, link_name));
                    vec![]
                }
                CoordinatorEvent::NodeDisconnected { .. } => {
                    vec![CoordinatorAction::AbortSimulation {
                        reason: "Node disconnected in AwaitingNodes".into(),
                    }]
                }
                CoordinatorEvent::QuantumDone { .. }
                | CoordinatorEvent::SimulationMessage { .. } => {
                    vec![CoordinatorAction::AbortSimulation {
                        reason: "Protocol violation in AwaitingNodes".into(),
                    }]
                }
            },
            Phase::AwaitingLinks { remaining } => match event {
                CoordinatorEvent::LinkRegister { node_id, link_name } => {
                    if let Some(link_cfg) = self.config.links.get(&link_name) {
                        remaining.remove(&(node_id, link_name.clone()));
                        let mut actions = vec![CoordinatorAction::SendLinkAck {
                            node_id,
                            link_id: link_cfg.link_id,
                        }];

                        if remaining.is_empty() {
                            let mut node_batches = HashMap::new();
                            for i in 0..self.config.expected_nodes {
                                node_batches.insert(i, Vec::new());
                            }
                            self.phase = Phase::Simulation {
                                node_batches,
                                barrier: BarrierState::new(
                                    self.config.expected_nodes as usize,
                                    self.config.max_messages_per_node,
                                ),
                            };
                            actions.push(CoordinatorAction::BroadcastClockStart {
                                release_quantum: 0,
                            });
                        }
                        actions
                    } else {
                        vec![CoordinatorAction::AbortSimulation {
                            reason: format!("Unknown link name: {}", link_name),
                        }]
                    }
                }
                CoordinatorEvent::NodeDisconnected { .. } => {
                    vec![CoordinatorAction::AbortSimulation {
                        reason: "Node disconnected in AwaitingLinks".into(),
                    }]
                }
                CoordinatorEvent::NodeJoined { .. } => {
                    vec![]
                }
                CoordinatorEvent::QuantumDone { .. }
                | CoordinatorEvent::SimulationMessage { .. } => {
                    vec![CoordinatorAction::AbortSimulation {
                        reason: "Protocol violation in AwaitingLinks".into(),
                    }]
                }
            },
            Phase::Simulation {
                node_batches,
                barrier,
            } => match event {
                CoordinatorEvent::SimulationMessage {
                    src_node_id,
                    link_id,
                    delivery_vtime_ns,
                    sequence_number,
                    payload,
                } => {
                    if let Some(batch) = node_batches.get_mut(&src_node_id) {
                        batch.push(PendingMessage {
                            src_node_id,
                            link_id,
                            delivery_vtime_ns,
                            sequence_number,
                            payload,
                        });
                        vec![]
                    } else {
                        vec![CoordinatorAction::AbortSimulation {
                            reason: "Unknown source node for simulation message".into(),
                        }]
                    }
                }
                CoordinatorEvent::QuantumDone {
                    node_id,
                    quantum,
                    vtime_ns,
                } => {
                    if let Some(batch) = node_batches.get_mut(&node_id) {
                        let mut current = Vec::new();
                        let mut future = Vec::new();
                        for msg in std::mem::take(batch) {
                            if msg.delivery_vtime_ns <= vtime_ns {
                                current.push(msg);
                            } else {
                                future.push(msg);
                            }
                        }
                        *batch = future;

                        match barrier.submit_done(node_id, quantum, current) {
                            Ok(None) => vec![],
                            Ok(Some(sorted_msgs)) => {
                                let mut actions = Vec::new();
                                for msg in sorted_msgs {
                                    let target_nodes = self
                                        .config
                                        .links
                                        .values()
                                        .find(|l| l.link_id == msg.link_id)
                                        .map(|l| l.target_nodes.clone())
                                        .unwrap_or_default();

                                    let mut targets: Vec<u32> = target_nodes
                                        .into_iter()
                                        .filter(|&t| t != msg.src_node_id)
                                        .collect();
                                    targets.sort_unstable();
                                    if !targets.is_empty() {
                                        actions.push(CoordinatorAction::RouteMessage {
                                            target_nodes: targets,
                                            link_id: msg.link_id,
                                            delivery_vtime_ns: msg.delivery_vtime_ns,
                                            sequence_number: msg.sequence_number,
                                            payload: msg.payload,
                                        });
                                    }
                                }
                                actions.push(CoordinatorAction::BroadcastClockStart {
                                    release_quantum: barrier.current_quantum,
                                });
                                actions
                            }
                            Err(e) => {
                                vec![CoordinatorAction::AbortSimulation {
                                    reason: format!("Barrier error: {:?}", e),
                                }]
                            }
                        }
                    } else {
                        vec![CoordinatorAction::AbortSimulation {
                            reason: "Unknown node in quantum done".into(),
                        }]
                    }
                }
                CoordinatorEvent::NodeDisconnected { .. } => {
                    vec![CoordinatorAction::AbortSimulation {
                        reason: "Node disconnected in Simulation".into(),
                    }]
                }
                CoordinatorEvent::LinkRegister { node_id, link_name } => {
                    if let Some(link_cfg) = self.config.links.get(&link_name) {
                        vec![CoordinatorAction::SendLinkAck {
                            node_id,
                            link_id: link_cfg.link_id,
                        }]
                    } else {
                        vec![CoordinatorAction::AbortSimulation {
                            reason: format!("Unknown link name: {}", link_name),
                        }]
                    }
                }
                CoordinatorEvent::NodeJoined { .. } => {
                    vec![]
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_config() -> CoordinatorConfig {
        let mut links = HashMap::new();
        links.insert(
            "link0".into(),
            LinkConfig {
                link_id: 10,
                target_nodes: vec![0, 1],
                delay_ns: 0,
            },
        );
        CoordinatorConfig {
            expected_nodes: 2,
            links,
            max_messages_per_node: 10,
        }
    }

    #[test]
    fn test_full_handshake_two_nodes() {
        let mut state = CoordinatorState::new(create_config());

        // Node 0 joins
        let actions = state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        assert_eq!(actions, vec![]);

        // Node 1 joins -> transitions to AwaitingLinks
        let actions = state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
        assert_eq!(actions, vec![]);

        // Node 0 registers link0
        let actions = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "link0".into(),
        });
        assert_eq!(
            actions,
            vec![CoordinatorAction::SendLinkAck {
                node_id: 0,
                link_id: 10
            }]
        );

        // Node 1 registers link0 -> finishes AwaitingLinks
        let actions = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 1,
            link_name: "link0".into(),
        });
        assert_eq!(
            actions,
            vec![
                CoordinatorAction::SendLinkAck {
                    node_id: 1,
                    link_id: 10
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 0 }
            ]
        );
    }

    #[test]
    fn test_early_link_register_buffered_and_replayed() {
        let mut state = CoordinatorState::new(create_config());

        // Node 0 registers link0 BEFORE joining
        let actions = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "link0".into(),
        });
        assert_eq!(actions, vec![]);

        // Node 0 joins
        let actions = state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        assert_eq!(actions, vec![]);

        // Node 1 joins -> transitions, replays buffered LinkRegister for node 0
        let actions = state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
        assert_eq!(
            actions,
            vec![CoordinatorAction::SendLinkAck {
                node_id: 0,
                link_id: 10
            }]
        );
    }

    #[test]
    fn test_all_early_link_registers_trigger_clock_start_on_last_join() {
        let mut state = CoordinatorState::new(create_config());

        state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "link0".into(),
        });
        state.apply(CoordinatorEvent::LinkRegister {
            node_id: 1,
            link_name: "link0".into(),
        });

        state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });

        let actions = state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
        assert_eq!(
            actions,
            vec![
                CoordinatorAction::SendLinkAck {
                    node_id: 0,
                    link_id: 10
                },
                CoordinatorAction::SendLinkAck {
                    node_id: 1,
                    link_id: 10
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 0 }
            ]
        );
    }

    fn setup_simulation_phase() -> CoordinatorState {
        let mut state = CoordinatorState::new(create_config());
        state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
        state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "link0".into(),
        });
        state.apply(CoordinatorEvent::LinkRegister {
            node_id: 1,
            link_name: "link0".into(),
        });
        state
    }

    #[test]
    fn test_barrier_withholds_clock_start_until_all_nodes_done() {
        let mut state = setup_simulation_phase();

        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert_eq!(actions, vec![]);

        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert_eq!(
            actions,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 1 }]
        );
    }

    #[test]
    fn test_three_consecutive_quanta() {
        let mut state = setup_simulation_phase();

        for q in 0..3 {
            let actions = state.apply(CoordinatorEvent::QuantumDone {
                node_id: 0,
                quantum: q,
                vtime_ns: (q + 1) * 1000,
            });
            assert_eq!(actions, vec![]);

            let actions = state.apply(CoordinatorEvent::QuantumDone {
                node_id: 1,
                quantum: q,
                vtime_ns: (q + 1) * 1000,
            });
            assert_eq!(
                actions,
                vec![CoordinatorAction::BroadcastClockStart {
                    release_quantum: q + 1
                }]
            );
        }
    }

    #[test]
    fn test_message_buffered_then_routed_at_quantum_boundary() {
        let mut state = setup_simulation_phase();

        let actions = state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 10,
            delivery_vtime_ns: 500,
            sequence_number: 1,
            payload: vec![1, 2, 3],
        });
        assert_eq!(actions, vec![]);

        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });

        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert_eq!(
            actions,
            vec![
                CoordinatorAction::RouteMessage {
                    target_nodes: vec![1],
                    link_id: 10,
                    delivery_vtime_ns: 500,
                    sequence_number: 1,
                    payload: vec![1, 2, 3],
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 1 }
            ]
        );
    }

    #[test]
    fn test_future_message_held_across_quantum_boundary() {
        let mut state = setup_simulation_phase();

        state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 10,
            delivery_vtime_ns: 2000,
            sequence_number: 1,
            payload: vec![1],
        });

        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert_eq!(
            actions,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 1 }]
        );

        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 1,
            vtime_ns: 2000,
        });
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 1,
            vtime_ns: 2000,
        });
        assert_eq!(
            actions,
            vec![
                CoordinatorAction::RouteMessage {
                    target_nodes: vec![1],
                    link_id: 10,
                    delivery_vtime_ns: 2000,
                    sequence_number: 1,
                    payload: vec![1],
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 2 }
            ]
        );
    }

    #[test]
    fn test_canonical_sort_same_vtime_by_src_then_seq() {
        let mut config = create_config();
        config.expected_nodes = 3;
        config.links.get_mut("link0").unwrap().target_nodes = vec![0, 1, 2];
        let mut state = CoordinatorState::new(config);

        for i in 0..3 {
            state.apply(CoordinatorEvent::NodeJoined { node_id: i });
        }
        for i in 0..3 {
            state.apply(CoordinatorEvent::LinkRegister {
                node_id: i,
                link_name: "link0".into(),
            });
        }

        // Node 1 seq 2
        state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 1,
            link_id: 10,
            delivery_vtime_ns: 1000,
            sequence_number: 2,
            payload: vec![1],
        });
        // Node 0 seq 1
        state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 10,
            delivery_vtime_ns: 1000,
            sequence_number: 1,
            payload: vec![2],
        });
        // Node 1 seq 1
        state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 1,
            link_id: 10,
            delivery_vtime_ns: 1000,
            sequence_number: 1,
            payload: vec![3],
        });

        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });
        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1000,
        });
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 2,
            quantum: 0,
            vtime_ns: 1000,
        });

        assert_eq!(actions.len(), 4); // 3 routes + 1 broadcast
        match &actions[0] {
            CoordinatorAction::RouteMessage {
                sequence_number,
                ..
            } => assert_eq!(*sequence_number, 1),
            _ => panic!("Expected route"),
        }
        match &actions[1] {
            CoordinatorAction::RouteMessage {
                sequence_number,
                ..
            } => assert_eq!(*sequence_number, 1),
            _ => panic!("Expected route"),
        }
        match &actions[2] {
            CoordinatorAction::RouteMessage {
                sequence_number,
                ..
            } => assert_eq!(*sequence_number, 2),
            _ => panic!("Expected route"),
        }
    }

    #[test]
    fn test_node_disconnected_aborts_in_simulation_phase() {
        let mut state = setup_simulation_phase();
        let actions = state.apply(CoordinatorEvent::NodeDisconnected { node_id: 0 });
        assert!(matches!(
            actions[0],
            CoordinatorAction::AbortSimulation { .. }
        ));
    }

    #[test]
    fn test_node_disconnected_aborts_in_awaiting_nodes_phase() {
        let mut state = CoordinatorState::new(create_config());
        let actions = state.apply(CoordinatorEvent::NodeDisconnected { node_id: 0 });
        assert!(matches!(
            actions[0],
            CoordinatorAction::AbortSimulation { .. }
        ));
    }

    #[test]
    fn test_unknown_link_name_aborts() {
        let mut state = CoordinatorState::new(create_config());
        state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
        let actions = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "nonexistent".into(),
        });
        assert!(matches!(
            actions[0],
            CoordinatorAction::AbortSimulation { .. }
        ));
    }

    #[test]
    fn test_duplicate_quantum_done_aborts() {
        let mut state = setup_simulation_phase();
        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert!(matches!(
            actions[0],
            CoordinatorAction::AbortSimulation { .. }
        ));
    }

    #[test]
    fn test_quantum_done_outside_simulation_phase_aborts() {
        let mut state = CoordinatorState::new(create_config());
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert!(matches!(
            actions[0],
            CoordinatorAction::AbortSimulation { .. }
        ));
    }

    #[test]
    fn test_simulation_message_outside_simulation_phase_aborts() {
        let mut state = CoordinatorState::new(create_config());
        let actions = state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 10,
            delivery_vtime_ns: 1000,
            sequence_number: 1,
            payload: vec![],
        });
        assert!(matches!(
            actions[0],
            CoordinatorAction::AbortSimulation { .. }
        ));
    }

    #[test]
    fn test_lookahead_node_completes_barrier_on_next_quantum() {
        let mut state = setup_simulation_phase();

        // Node 0 submits q=1 (lookahead)
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 1,
            vtime_ns: 2000,
        });
        assert_eq!(actions, vec![]);

        // Node 1 submits q=0
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert_eq!(actions, vec![]); // Needs node 0 for q=0

        // Node 0 submits q=0 -> releases q=0
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1000,
        });
        assert_eq!(
            actions,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 1 }]
        );

        // Node 1 submits q=1 -> releases q=1 immediately since node 0 already did
        let actions = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 1,
            vtime_ns: 2000,
        });
        assert_eq!(
            actions,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 2 }]
        );
    }
}

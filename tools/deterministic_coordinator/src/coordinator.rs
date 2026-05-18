#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"

use std::collections::{HashMap, HashSet};

// ── Public configuration (DI — no env vars, no globals) ─────────────────────

#[derive(Debug, Clone)]
pub struct LinkConfig {
    pub link_id: u32,
    pub target_nodes: Vec<u32>,
    pub delay_ns: u64,
}

#[derive(Debug)]
pub struct CoordinatorConfig {
    pub expected_nodes: u32,
    /// Keyed by link name (e.g., "can0"). Injected from topology at startup.
    pub links: HashMap<String, LinkConfig>,
    pub max_messages_per_node: usize,
}

// ── Events (I/O boundary parses wire bytes → typed events) ──────────────────

#[derive(Debug)]
pub enum CoordinatorEvent {
    NodeJoined { node_id: u32 },
    NodeDisconnected { node_id: u32 },
    LinkRegister { node_id: u32, link_name: String },
    QuantumDone { node_id: u32, quantum: u64, vtime_ns: u64 },
    SimulationMessage {
        src_node_id: u32,
        link_id: u32,
        delivery_vtime_ns: u64,
        sequence_number: u64,
        payload: Vec<u8>,
    },
}

// ── Actions (I/O boundary executes these on sockets/Zenoh) ──────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum CoordinatorAction {
    SendLinkAck { node_id: u32, link_id: u32 },
    BroadcastClockStart { release_quantum: u64 },
    RouteMessage {
        /// Sorted ascending. Source node is excluded.
        target_nodes: Vec<u32>,
        link_id: u32,
        delivery_vtime_ns: u64,
        sequence_number: u64,
        payload: Vec<u8>,
    },
    AbortSimulation { reason: String },
}

// ── Internal message representation ─────────────────────────────────────────

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
        // Canonical PDES tie-breaking: (delivery_vtime_ns, src_node_id, sequence_number)
        self.delivery_vtime_ns
            .cmp(&other.delivery_vtime_ns)
            .then_with(|| self.src_node_id.cmp(&other.src_node_id))
            .then_with(|| self.sequence_number.cmp(&other.sequence_number))
    }
}

// ── Synchronous barrier (plain data — no Mutex, no Condvar) ─────────────────

#[derive(Debug)]
enum BarrierError {
    DuplicateDone,
    NodeIndexOutOfBounds(u32),
    QuantumMismatch { expected: u64, got: u64 },
}

struct BarrierState {
    n_nodes: usize,
    max_messages_per_node: usize,
    current_quantum: u64,
    done_nodes: Vec<bool>,
    message_buffer: Vec<PendingMessage>,
    // One-quantum lookahead: nodes that submitted Q+1 while Q is still open.
    next_quantum_done_nodes: Vec<bool>,
    next_quantum_buffer: Vec<PendingMessage>,
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
        let current = self.current_quantum;

        if quantum < current {
            return Ok(None); // stale; already advanced past this quantum
        }

        if node_id as usize >= self.n_nodes {
            return Err(BarrierError::NodeIndexOutOfBounds(node_id));
        }

        if quantum > current + 1 {
            return Err(BarrierError::QuantumMismatch {
                expected: current,
                got: quantum,
            });
        }

        // One-quantum lookahead: node is already done with Q+1.
        if quantum == current + 1 {
            if self.next_quantum_done_nodes[node_id as usize] {
                return Err(BarrierError::DuplicateDone);
            }
            self.next_quantum_done_nodes[node_id as usize] = true;
            messages.sort();
            if messages.len() > self.max_messages_per_node {
                messages.truncate(self.max_messages_per_node);
            }
            self.next_quantum_buffer.extend(messages);
            if self.next_quantum_done_nodes.iter().all(|&d| d) {
                // Every node landed in lookahead — barrier will never release.
                // Most common cause: pre-increment bug in the test runner or I/O layer.
                tracing::error!(
                    "QuantumBarrier: ALL {} nodes in LOOKAHEAD for quantum={} (current={}). \
                     Pre-increment bug suspected — capture quantum BEFORE step_clock(), \
                     increment AFTER.",
                    self.n_nodes,
                    quantum,
                    current
                );
            }
            return Ok(None);
        }

        // Current quantum submission.
        if self.done_nodes[node_id as usize] {
            return Err(BarrierError::DuplicateDone);
        }
        self.done_nodes[node_id as usize] = true;

        messages.sort();
        if messages.len() > self.max_messages_per_node {
            messages.truncate(self.max_messages_per_node);
        }
        self.message_buffer.extend(messages);

        let done_count = self.done_nodes.iter().filter(|&&d| d).count();
        if done_count < self.n_nodes {
            return Ok(None);
        }

        // All nodes done. Sort and promote lookahead.
        let mut all_msgs = std::mem::take(&mut self.message_buffer);
        all_msgs.sort();

        self.current_quantum += 1;
        self.message_buffer = std::mem::take(&mut self.next_quantum_buffer);
        self.done_nodes = std::mem::take(&mut self.next_quantum_done_nodes);
        self.next_quantum_done_nodes = vec![false; self.n_nodes];

        Ok(Some(all_msgs))
    }
}

// ── Explicit phase model ─────────────────────────────────────────────────────

enum Phase {
    AwaitingNodes {
        joined: HashSet<u32>,
        /// LinkRegister events that arrive before all nodes join are buffered here.
        buffered_link_regs: Vec<(u32, String)>,
    },
    AwaitingLinks {
        /// (node_id, link_name) pairs still expecting registration.
        remaining: HashSet<(u32, String)>,
    },
    Simulation {
        barrier: BarrierState,
        /// Per-source-node message batches, partitioned at QuantumDone time.
        node_batches: HashMap<u32, Vec<PendingMessage>>,
    },
}

// ── State machine ────────────────────────────────────────────────────────────

pub struct CoordinatorState {
    config: CoordinatorConfig,
    phase: Phase,
}

impl CoordinatorState {
    pub fn new(config: CoordinatorConfig) -> Self {
        Self {
            phase: Phase::AwaitingNodes {
                joined: HashSet::new(),
                buffered_link_regs: Vec::new(),
            },
            config,
        }
    }

    pub fn apply(&mut self, event: CoordinatorEvent) -> Vec<CoordinatorAction> {
        match event {
            CoordinatorEvent::NodeJoined { node_id } => self.on_node_joined(node_id),
            CoordinatorEvent::NodeDisconnected { node_id } => self.on_node_disconnected(node_id),
            CoordinatorEvent::LinkRegister { node_id, link_name } => {
                self.on_link_register(node_id, link_name)
            }
            CoordinatorEvent::QuantumDone {
                node_id,
                quantum,
                vtime_ns,
            } => self.on_quantum_done(node_id, quantum, vtime_ns),
            CoordinatorEvent::SimulationMessage {
                src_node_id,
                link_id,
                delivery_vtime_ns,
                sequence_number,
                payload,
            } => self.on_simulation_message(
                src_node_id,
                link_id,
                delivery_vtime_ns,
                sequence_number,
                payload,
            ),
        }
    }

    fn on_node_joined(&mut self, node_id: u32) -> Vec<CoordinatorAction> {
        // Extract buffered link registrations if this was the final node joining.
        let buffered = match &mut self.phase {
            Phase::AwaitingNodes {
                joined,
                buffered_link_regs,
            } => {
                joined.insert(node_id);
                if joined.len() == self.config.expected_nodes as usize {
                    Some(std::mem::take(buffered_link_regs))
                } else {
                    None
                }
            }
            // Late join during later phases: ignore (no re-registration supported).
            _ => return Vec::new(),
        };
        // Borrow of self.phase ends here.

        let Some(buffered) = buffered else {
            return Vec::new();
        };

        // Build the complete set of (node_id, link_name) pairs we expect to register.
        let remaining: HashSet<(u32, String)> = self
            .config
            .links
            .iter()
            .flat_map(|(name, cfg)| {
                cfg.target_nodes
                    .iter()
                    .map(move |&nid| (nid, name.clone()))
            })
            .collect();

        self.phase = Phase::AwaitingLinks { remaining };

        // Replay any link registrations that arrived early.
        let mut actions = Vec::new();
        for (buf_node_id, buf_link_name) in buffered {
            actions.extend(self.process_link_register(buf_node_id, buf_link_name));
        }
        actions
    }

    fn on_node_disconnected(&mut self, node_id: u32) -> Vec<CoordinatorAction> {
        // Crash-only: any disconnection aborts the simulation regardless of phase.
        vec![CoordinatorAction::AbortSimulation {
            reason: format!("node {node_id} disconnected"),
        }]
    }

    fn on_link_register(&mut self, node_id: u32, link_name: String) -> Vec<CoordinatorAction> {
        // Buffer during join phase; the NLL borrow ends before the delegation below.
        if let Phase::AwaitingNodes {
            buffered_link_regs, ..
        } = &mut self.phase
        {
            buffered_link_regs.push((node_id, link_name));
            return Vec::new();
        }
        // Borrow of self.phase ends here.
        self.process_link_register(node_id, link_name)
    }

    fn process_link_register(&mut self, node_id: u32, link_name: String) -> Vec<CoordinatorAction> {
        let link_id = match self.config.links.get(&link_name) {
            Some(cfg) => cfg.link_id,
            None => {
                return vec![CoordinatorAction::AbortSimulation {
                    reason: format!(
                        "node {node_id} registered unknown link '{link_name}'"
                    ),
                }]
            }
        };

        // Check whether this registration completes the preflight set.
        let should_start = match &mut self.phase {
            Phase::AwaitingLinks { remaining } => {
                remaining.remove(&(node_id, link_name));
                remaining.is_empty()
            }
            // Late registration after simulation started: ack but do not change state.
            _ => false,
        };
        // Borrow of self.phase ends here.

        let mut actions = vec![CoordinatorAction::SendLinkAck { node_id, link_id }];

        if should_start {
            let n_nodes = self.config.expected_nodes as usize;
            let max_msgs = self.config.max_messages_per_node;
            self.phase = Phase::Simulation {
                barrier: BarrierState::new(n_nodes, max_msgs),
                node_batches: HashMap::new(),
            };
            actions.push(CoordinatorAction::BroadcastClockStart { release_quantum: 0 });
        }

        actions
    }

    fn on_quantum_done(
        &mut self,
        node_id: u32,
        quantum: u64,
        vtime_ns: u64,
    ) -> Vec<CoordinatorAction> {
        let (barrier_result, release_quantum) = match &mut self.phase {
            Phase::Simulation {
                barrier,
                node_batches,
            } => {
                let pending = node_batches.remove(&node_id).unwrap_or_default();
                // Partition: messages deliverable in this quantum vs future quanta.
                let (current_msgs, future_msgs): (Vec<_>, Vec<_>) =
                    pending.into_iter().partition(|m| m.delivery_vtime_ns <= vtime_ns);
                if !future_msgs.is_empty() {
                    node_batches.insert(node_id, future_msgs);
                }
                let result = barrier.submit_done(node_id, quantum, current_msgs);
                let q = barrier.current_quantum;
                (result, q)
            }
            _ => {
                return vec![CoordinatorAction::AbortSimulation {
                    reason: format!(
                        "QuantumDone from node {node_id} received outside Simulation phase"
                    ),
                }]
            }
        };
        // Borrow of self.phase ends here; self.config is now freely accessible.

        match barrier_result {
            Ok(Some(sorted_msgs)) => {
                let mut actions: Vec<CoordinatorAction> = sorted_msgs
                    .into_iter()
                    .filter_map(|msg| {
                        let link_cfg = self
                            .config
                            .links
                            .values()
                            .find(|c| c.link_id == msg.link_id)?;
                        let mut target_nodes: Vec<u32> = link_cfg
                            .target_nodes
                            .iter()
                            .copied()
                            .filter(|&n| n != msg.src_node_id)
                            .collect();
                        target_nodes.sort_unstable();
                        if target_nodes.is_empty() {
                            return None;
                        }
                        Some(CoordinatorAction::RouteMessage {
                            target_nodes,
                            link_id: msg.link_id,
                            delivery_vtime_ns: msg.delivery_vtime_ns,
                            sequence_number: msg.sequence_number,
                            payload: msg.payload,
                        })
                    })
                    .collect();
                actions.push(CoordinatorAction::BroadcastClockStart { release_quantum });
                actions
            }
            Ok(None) => Vec::new(),
            Err(e) => vec![CoordinatorAction::AbortSimulation {
                reason: format!("barrier error for node {node_id}: {e:?}"),
            }],
        }
    }

    fn on_simulation_message(
        &mut self,
        src_node_id: u32,
        link_id: u32,
        delivery_vtime_ns: u64,
        sequence_number: u64,
        payload: Vec<u8>,
    ) -> Vec<CoordinatorAction> {
        match &mut self.phase {
            Phase::Simulation { node_batches, .. } => {
                node_batches
                    .entry(src_node_id)
                    .or_default()
                    .push(PendingMessage {
                        src_node_id,
                        link_id,
                        delivery_vtime_ns,
                        sequence_number,
                        payload,
                    });
                Vec::new()
            }
            _ => vec![CoordinatorAction::AbortSimulation {
                reason: format!(
                    "SimulationMessage from node {src_node_id} received outside Simulation phase"
                ),
            }],
        }
    }
}

// ── Unit tests (TDD: written before implementation, drive the design) ────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn two_node_one_link_config() -> CoordinatorConfig {
        let mut links = HashMap::new();
        links.insert(
            "can0".to_string(),
            LinkConfig {
                link_id: 0,
                target_nodes: vec![0, 1],
                delay_ns: 0,
            },
        );
        CoordinatorConfig {
            expected_nodes: 2,
            links,
            max_messages_per_node: 1024,
        }
    }

    /// Drive a 2-node coordinator through join + link-registration up to
    /// the first `BroadcastClockStart { release_quantum: 0 }`.
    fn complete_handshake(state: &mut CoordinatorState) {
        let a = state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        assert_eq!(a, vec![], "first join: no action");

        let a = state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
        assert_eq!(a, vec![], "second join: all nodes present but no links yet");

        let a = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "can0".to_string(),
        });
        assert_eq!(
            a,
            vec![CoordinatorAction::SendLinkAck {
                node_id: 0,
                link_id: 0
            }],
            "first link reg: ack only"
        );

        let a = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 1,
            link_name: "can0".to_string(),
        });
        assert_eq!(
            a,
            vec![
                CoordinatorAction::SendLinkAck {
                    node_id: 1,
                    link_id: 0
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 0 },
            ],
            "last link reg: ack + clock start"
        );
    }

    // ── Phase 1: Join ─────────────────────────────────────────────────────────

    #[test]
    fn test_full_handshake_two_nodes() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);
    }

    #[test]
    fn test_early_link_register_buffered_and_replayed() {
        let mut state = CoordinatorState::new(two_node_one_link_config());

        // Node 0's peripheral initializes and calls register_link before all nodes join.
        let early = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "can0".to_string(),
        });
        assert_eq!(early, vec![], "buffered: no ack during AwaitingNodes");

        let a = state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        assert_eq!(a, vec![], "first join");

        // Joining the second node triggers phase transition and replays the buffer.
        let a = state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
        assert_eq!(
            a,
            vec![CoordinatorAction::SendLinkAck {
                node_id: 0,
                link_id: 0
            }],
            "phase transition replays buffered registration"
        );

        // Now node 1 registers normally.
        let a = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 1,
            link_name: "can0".to_string(),
        });
        assert_eq!(
            a,
            vec![
                CoordinatorAction::SendLinkAck {
                    node_id: 1,
                    link_id: 0
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 0 },
            ]
        );
    }

    #[test]
    fn test_all_early_link_registers_trigger_clock_start_on_last_join() {
        let mut state = CoordinatorState::new(two_node_one_link_config());

        // Both nodes register links before either has officially joined.
        state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "can0".to_string(),
        });
        state.apply(CoordinatorEvent::LinkRegister {
            node_id: 1,
            link_name: "can0".to_string(),
        });

        state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        let a = state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });

        // Both acks + clock start must all be returned from the final NodeJoined.
        assert!(
            a.contains(&CoordinatorAction::SendLinkAck {
                node_id: 0,
                link_id: 0
            }),
            "ack for node 0"
        );
        assert!(
            a.contains(&CoordinatorAction::SendLinkAck {
                node_id: 1,
                link_id: 0
            }),
            "ack for node 1"
        );
        assert!(
            a.contains(&CoordinatorAction::BroadcastClockStart { release_quantum: 0 }),
            "clock start"
        );
    }

    // ── Phase 3: Simulation barrier ───────────────────────────────────────────

    #[test]
    fn test_barrier_withholds_clock_start_until_all_nodes_done() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1_000,
        });
        assert_eq!(a, vec![], "first done: barrier not released");

        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1_000,
        });
        assert_eq!(
            a,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 1 }],
            "second done: barrier releases quantum 1"
        );
    }

    #[test]
    fn test_three_consecutive_quanta() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        for quantum in 0u64..3 {
            let a = state.apply(CoordinatorEvent::QuantumDone {
                node_id: 0,
                quantum,
                vtime_ns: (quantum + 1) * 1_000,
            });
            assert_eq!(a, vec![], "quantum {quantum}: node 0 done, waiting");

            let a = state.apply(CoordinatorEvent::QuantumDone {
                node_id: 1,
                quantum,
                vtime_ns: (quantum + 1) * 1_000,
            });
            assert_eq!(
                a,
                vec![CoordinatorAction::BroadcastClockStart {
                    release_quantum: quantum + 1
                }],
                "quantum {quantum}: released"
            );
        }
    }

    // ── Message routing ───────────────────────────────────────────────────────

    #[test]
    fn test_message_buffered_then_routed_at_quantum_boundary() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        // Node 0 sends a message before its QuantumDone.
        let a = state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 0,
            delivery_vtime_ns: 500,
            sequence_number: 1,
            payload: vec![0xDE, 0xAD],
        });
        assert_eq!(a, vec![], "message buffered");

        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1_000,
        });
        assert_eq!(a, vec![], "waiting for node 1");

        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1_000,
        });
        // Source node (0) excluded from target_nodes.
        assert_eq!(
            a,
            vec![
                CoordinatorAction::RouteMessage {
                    target_nodes: vec![1],
                    link_id: 0,
                    delivery_vtime_ns: 500,
                    sequence_number: 1,
                    payload: vec![0xDE, 0xAD],
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 1 },
            ]
        );
    }

    #[test]
    fn test_future_message_held_across_quantum_boundary() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        // Message with delivery_vtime_ns beyond this quantum's end.
        state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 0,
            delivery_vtime_ns: 2_000,
            sequence_number: 1,
            payload: vec![0xFF],
        });

        // Quantum 0 ends at vtime=1000; message at 2000 is future.
        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1_000,
        });
        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1_000,
        });
        assert_eq!(
            a,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 1 }],
            "future message must NOT be routed in quantum 0"
        );

        // Quantum 1 ends at vtime=2000; message is now deliverable.
        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 1,
            vtime_ns: 2_000,
        });
        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 1,
            vtime_ns: 2_000,
        });
        assert_eq!(
            a,
            vec![
                CoordinatorAction::RouteMessage {
                    target_nodes: vec![1],
                    link_id: 0,
                    delivery_vtime_ns: 2_000,
                    sequence_number: 1,
                    payload: vec![0xFF],
                },
                CoordinatorAction::BroadcastClockStart { release_quantum: 2 },
            ],
            "held message must be routed in quantum 1"
        );
    }

    #[test]
    fn test_canonical_sort_same_vtime_by_src_then_seq() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        // Three messages at identical delivery_vtime_ns from different sources/sequences.
        // The coordinator must deliver them in (delivery_vtime_ns, src_node_id, seq) order.
        // Using a 3-node config for this test.
        let mut links = HashMap::new();
        links.insert(
            "bus".to_string(),
            LinkConfig {
                link_id: 0,
                target_nodes: vec![0, 1, 2],
                delay_ns: 0,
            },
        );
        let config = CoordinatorConfig {
            expected_nodes: 3,
            links,
            max_messages_per_node: 1024,
        };
        let mut state3 = CoordinatorState::new(config);

        for node_id in 0..3 {
            state3.apply(CoordinatorEvent::NodeJoined { node_id });
        }
        for node_id in 0..3 {
            state3.apply(CoordinatorEvent::LinkRegister {
                node_id,
                link_name: "bus".to_string(),
            });
        }

        // Each node sends one message at the same vtime.
        state3.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 2,
            link_id: 0,
            delivery_vtime_ns: 100,
            sequence_number: 0,
            payload: vec![2],
        });
        state3.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 0,
            delivery_vtime_ns: 100,
            sequence_number: 0,
            payload: vec![0],
        });
        state3.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 1,
            link_id: 0,
            delivery_vtime_ns: 100,
            sequence_number: 0,
            payload: vec![1],
        });

        state3.apply(CoordinatorEvent::QuantumDone { node_id: 2, quantum: 0, vtime_ns: 1_000 });
        state3.apply(CoordinatorEvent::QuantumDone { node_id: 0, quantum: 0, vtime_ns: 1_000 });
        let actions = state3.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1_000,
        });

        let route_payloads: Vec<Vec<u8>> = actions
            .iter()
            .filter_map(|a| {
                if let CoordinatorAction::RouteMessage { payload, .. } = a {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .collect();

        // Must be sorted by src_node_id (0, 1, 2) since vtime and seq are equal.
        assert_eq!(route_payloads, vec![vec![0], vec![1], vec![2]]);
    }

    // ── Error paths ───────────────────────────────────────────────────────────

    #[test]
    fn test_node_disconnected_aborts_in_simulation_phase() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        let a = state.apply(CoordinatorEvent::NodeDisconnected { node_id: 0 });
        assert_eq!(a.len(), 1);
        assert!(matches!(a[0], CoordinatorAction::AbortSimulation { .. }));
    }

    #[test]
    fn test_node_disconnected_aborts_in_awaiting_nodes_phase() {
        let mut state = CoordinatorState::new(two_node_one_link_config());

        let a = state.apply(CoordinatorEvent::NodeDisconnected { node_id: 0 });
        assert!(matches!(a[0], CoordinatorAction::AbortSimulation { .. }));
    }

    #[test]
    fn test_unknown_link_name_aborts() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
        state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });

        let a = state.apply(CoordinatorEvent::LinkRegister {
            node_id: 0,
            link_name: "nonexistent".to_string(),
        });
        assert_eq!(a.len(), 1);
        assert!(matches!(a[0], CoordinatorAction::AbortSimulation { .. }));
    }

    #[test]
    fn test_duplicate_quantum_done_aborts() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1_000,
        });

        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1_000,
        });
        assert!(matches!(a[0], CoordinatorAction::AbortSimulation { .. }));
    }

    #[test]
    fn test_quantum_done_outside_simulation_phase_aborts() {
        let mut state = CoordinatorState::new(two_node_one_link_config());

        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 0,
        });
        assert!(matches!(a[0], CoordinatorAction::AbortSimulation { .. }));
    }

    #[test]
    fn test_simulation_message_outside_simulation_phase_aborts() {
        let mut state = CoordinatorState::new(two_node_one_link_config());

        let a = state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 0,
            delivery_vtime_ns: 0,
            sequence_number: 0,
            payload: vec![],
        });
        assert!(matches!(a[0], CoordinatorAction::AbortSimulation { .. }));
    }

    // ── Lookahead ─────────────────────────────────────────────────────────────

    #[test]
    fn test_lookahead_node_completes_barrier_on_next_quantum() {
        let mut state = CoordinatorState::new(two_node_one_link_config());
        complete_handshake(&mut state);

        // Node 0 fast-forwards to quantum 1 while quantum 0 is still open.
        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 1,
            vtime_ns: 2_000,
        });
        assert_eq!(a, vec![], "lookahead: buffered, no release");

        // Node 1 finishes quantum 0 — barrier releases quantum 0.
        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 0,
            vtime_ns: 1_000,
        });
        // Node 0 already counted for quantum 0 (via stale check)? No —
        // lookahead node 0 submitted q=1, not q=0. Node 1 alone closes q=0.
        // But with 2 nodes required, q=0 can't close with only node 1.
        // So this must still return []. Node 0 must also submit q=0.
        assert_eq!(a, vec![], "node 0 in lookahead, cannot close q=0 with node 1 alone");

        // Node 0 now also submits quantum 0 — closes the barrier.
        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: 0,
            vtime_ns: 1_000,
        });
        assert_eq!(
            a,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 1 }],
            "quantum 0 releases"
        );

        // Quantum 1: node 0 is already in lookahead. Node 1 finishes q=1 → release.
        let a = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: 1,
            vtime_ns: 2_000,
        });
        assert_eq!(
            a,
            vec![CoordinatorAction::BroadcastClockStart { release_quantum: 2 }],
            "quantum 1 releases because node 0 was in lookahead"
        );
    }
}

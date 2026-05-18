/// Step 0.2.5 — Pure-Rust integration test (no QEMU, no sockets, no async).
///
/// Drives a complete 2-node protocol exchange through 3 quantum cycles using
/// only typed events and assertions on the returned action vectors.
/// This catches state-transition bugs and wire-protocol invariants before
/// any QEMU or real socket involvement.
use deterministic_coordinator::coordinator::{
    CoordinatorAction, CoordinatorConfig, CoordinatorEvent, CoordinatorState, LinkConfig,
};
use std::collections::HashMap;

fn two_node_two_link_config() -> CoordinatorConfig {
    let mut links = HashMap::new();
    links.insert(
        "link0".to_string(),
        LinkConfig {
            link_id: 0,
            target_nodes: vec![0, 1],
            delay_ns: 0,
        },
    );
    links.insert(
        "link1".to_string(),
        LinkConfig {
            link_id: 1,
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

/// Full 2-node, 2-link, 3-quantum exchange.
///
/// Node 0 and Node 1 alternate sending messages on different links.
/// The test asserts the exact `Vec<CoordinatorAction>` at every step.
#[test]
fn test_full_two_node_three_quantum_protocol() {
    let mut coord = CoordinatorState::new(two_node_two_link_config());

    // ── Phase 1: Join ─────────────────────────────────────────────────────

    assert_eq!(
        coord.apply(CoordinatorEvent::NodeJoined { node_id: 0 }),
        vec![],
        "join 0: no action"
    );
    assert_eq!(
        coord.apply(CoordinatorEvent::NodeJoined { node_id: 1 }),
        vec![],
        "join 1: no action (links not registered)"
    );

    // ── Phase 2: Preflight link registration ─────────────────────────────

    // 4 registrations required: (node0, link0), (node0, link1), (node1, link0), (node1, link1)
    let a = coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link0".to_string(),
    });
    assert_eq!(
        a,
        vec![CoordinatorAction::SendLinkAck {
            node_id: 0,
            link_id: 0
        }]
    );

    let a = coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link1".to_string(),
    });
    assert_eq!(
        a,
        vec![CoordinatorAction::SendLinkAck {
            node_id: 0,
            link_id: 1
        }]
    );

    let a = coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link0".to_string(),
    });
    assert_eq!(
        a,
        vec![CoordinatorAction::SendLinkAck {
            node_id: 1,
            link_id: 0
        }]
    );

    // Final registration triggers transition to Simulation.
    let a = coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link1".to_string(),
    });
    assert_eq!(
        a,
        vec![
            CoordinatorAction::SendLinkAck {
                node_id: 1,
                link_id: 1
            },
            CoordinatorAction::BroadcastClockStart { release_quantum: 0 },
        ],
        "last registration: ack + clock start for quantum 0"
    );

    // ── Phase 3: 3 quantum cycles ─────────────────────────────────────────

    for quantum in 0u64..3 {
        let vtime_ns = (quantum + 1) * 10_000;

        // Node 0 sends on link0, Node 1 sends on link1 (different links, same quantum).
        let msg0_seq = quantum * 10 + 1;
        let msg1_seq = quantum * 10 + 2;
        let msg0_vtime = vtime_ns / 2; // well within the quantum window
        let msg1_vtime = vtime_ns / 2;

        coord.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 0,
            delivery_vtime_ns: msg0_vtime,
            sequence_number: msg0_seq,
            payload: vec![0x00, quantum as u8],
        });
        coord.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 1,
            link_id: 1,
            delivery_vtime_ns: msg1_vtime,
            sequence_number: msg1_seq,
            payload: vec![0x01, quantum as u8],
        });

        // Node 0 finishes — barrier not yet complete.
        let a = coord.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum,
            vtime_ns,
        });
        assert_eq!(a, vec![], "quantum {quantum}, node 0 done: still waiting");

        // Node 1 finishes — barrier releases.
        let a = coord.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum,
            vtime_ns,
        });

        // Both messages should be routed (src excluded from targets).
        let route_actions: Vec<&CoordinatorAction> = a
            .iter()
            .filter(|x| matches!(x, CoordinatorAction::RouteMessage { .. }))
            .collect();
        assert_eq!(route_actions.len(), 2, "quantum {quantum}: 2 messages routed");

        // Message from node 0 on link0 → only node 1.
        assert!(
            a.contains(&CoordinatorAction::RouteMessage {
                target_nodes: vec![1],
                link_id: 0,
                delivery_vtime_ns: msg0_vtime,
                sequence_number: msg0_seq,
                payload: vec![0x00, quantum as u8],
            }),
            "quantum {quantum}: node 0's message routed to node 1"
        );

        // Message from node 1 on link1 → only node 0.
        assert!(
            a.contains(&CoordinatorAction::RouteMessage {
                target_nodes: vec![0],
                link_id: 1,
                delivery_vtime_ns: msg1_vtime,
                sequence_number: msg1_seq,
                payload: vec![0x01, quantum as u8],
            }),
            "quantum {quantum}: node 1's message routed to node 0"
        );

        // Clock start for next quantum is the final action.
        assert_eq!(
            a.last(),
            Some(&CoordinatorAction::BroadcastClockStart {
                release_quantum: quantum + 1
            }),
            "quantum {quantum}: clock start is last action"
        );

        // No abort actions.
        assert!(
            !a.iter()
                .any(|x| matches!(x, CoordinatorAction::AbortSimulation { .. })),
            "quantum {quantum}: no abort"
        );
    }
}

/// Verify that early link registrations (before all nodes join) are correctly
/// replayed when the final node joins — across a 2-node, 2-link topology.
#[test]
fn test_early_registrations_across_two_links() {
    let mut coord = CoordinatorState::new(two_node_two_link_config());

    // Both peripherals on node 0 register links before node 0 even joins.
    coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link0".to_string(),
    });
    coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link1".to_string(),
    });

    coord.apply(CoordinatorEvent::NodeJoined { node_id: 0 });

    // Node 1 joins — triggers replay of node 0's buffered registrations.
    let a = coord.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
    // Two acks for the buffered node-0 registrations.
    assert!(a.contains(&CoordinatorAction::SendLinkAck {
        node_id: 0,
        link_id: 0
    }));
    assert!(a.contains(&CoordinatorAction::SendLinkAck {
        node_id: 0,
        link_id: 1
    }));
    assert!(
        !a.contains(&CoordinatorAction::BroadcastClockStart { release_quantum: 0 }),
        "clock start must not fire until node 1 registers its links"
    );

    // Node 1 registers its two links — second one triggers clock start.
    let a1 = coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link0".to_string(),
    });
    assert_eq!(
        a1,
        vec![CoordinatorAction::SendLinkAck {
            node_id: 1,
            link_id: 0
        }]
    );

    let a2 = coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link1".to_string(),
    });
    assert!(a2.contains(&CoordinatorAction::BroadcastClockStart { release_quantum: 0 }));
}

/// Verify that a message sent in quantum N with a delivery_vtime past that
/// quantum's boundary is correctly deferred to quantum N+1 and delivered then.
#[test]
fn test_deferred_message_crosses_quantum_boundary() {
    let mut links = HashMap::new();
    links.insert(
        "bus".to_string(),
        LinkConfig {
            link_id: 0,
            target_nodes: vec![0, 1],
            delay_ns: 0,
        },
    );
    let mut coord = CoordinatorState::new(CoordinatorConfig {
        expected_nodes: 2,
        links,
        max_messages_per_node: 1024,
    });

    coord.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
    coord.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
    coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "bus".to_string(),
    });
    coord.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "bus".to_string(),
    });

    // Node 0 sends a message with delivery_vtime_ns = 15_000 while quantum 0 ends at 10_000.
    coord.apply(CoordinatorEvent::SimulationMessage {
        src_node_id: 0,
        link_id: 0,
        delivery_vtime_ns: 15_000,
        sequence_number: 42,
        payload: vec![0xAB],
    });

    // Close quantum 0 (vtime = 10_000): message is future → not delivered.
    coord.apply(CoordinatorEvent::QuantumDone { node_id: 0, quantum: 0, vtime_ns: 10_000 });
    let a_q0 = coord.apply(CoordinatorEvent::QuantumDone {
        node_id: 1,
        quantum: 0,
        vtime_ns: 10_000,
    });
    assert!(
        !a_q0
            .iter()
            .any(|x| matches!(x, CoordinatorAction::RouteMessage { .. })),
        "future message must not appear in quantum 0"
    );

    // Close quantum 1 (vtime = 20_000): message is now deliverable.
    coord.apply(CoordinatorEvent::QuantumDone { node_id: 0, quantum: 1, vtime_ns: 20_000 });
    let a_q1 = coord.apply(CoordinatorEvent::QuantumDone {
        node_id: 1,
        quantum: 1,
        vtime_ns: 20_000,
    });
    assert!(
        a_q1.contains(&CoordinatorAction::RouteMessage {
            target_nodes: vec![1],
            link_id: 0,
            delivery_vtime_ns: 15_000,
            sequence_number: 42,
            payload: vec![0xAB],
        }),
        "deferred message must be delivered in quantum 1"
    );
}

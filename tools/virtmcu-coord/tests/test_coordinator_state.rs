use virtmcu_coord::coordinator::{
    CoordinatorAction, CoordinatorConfig, CoordinatorEvent, CoordinatorState, LinkConfig,
};
use std::collections::HashMap;

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
    links.insert(
        "link1".into(),
        LinkConfig {
            link_id: 11,
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
fn test_full_two_node_three_quantum_protocol() {
    let mut state = CoordinatorState::new(create_config());

    state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
    state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
    state.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link0".into(),
    });
    state.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link1".into(),
    });
    state.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link0".into(),
    });
    let init_actions = state.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link1".into(),
    });

    assert!(matches!(
        init_actions.last().unwrap(),
        CoordinatorAction::BroadcastClockStart { release_quantum: 0 }
    ));

    for q in 0..3 {
        state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 0,
            link_id: 10,
            delivery_vtime_ns: (q + 1) * 1000 - 500,
            sequence_number: 1,
            payload: vec![q as u8],
        });
        state.apply(CoordinatorEvent::SimulationMessage {
            src_node_id: 1,
            link_id: 11,
            delivery_vtime_ns: (q + 1) * 1000 - 500,
            sequence_number: 1,
            payload: vec![q as u8 + 10],
        });

        let actions0 = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 0,
            quantum: q,
            vtime_ns: (q + 1) * 1000,
        });
        assert_eq!(actions0, vec![]);

        let actions1 = state.apply(CoordinatorEvent::QuantumDone {
            node_id: 1,
            quantum: q,
            vtime_ns: (q + 1) * 1000,
        });

        assert_eq!(actions1.len(), 3);

        let mut routes = vec![];
        for a in &actions1 {
            if let CoordinatorAction::RouteMessage {
                target_nodes,
                link_id,
                payload,
                ..
            } = a
            {
                routes.push((target_nodes.clone(), *link_id, payload.clone()));
            }
        }
        routes.sort_by_key(|r| r.1);

        assert_eq!(routes[0], (vec![1], 10, vec![q as u8]));
        assert_eq!(routes[1], (vec![0], 11, vec![q as u8 + 10]));

        assert!(matches!(
            actions1.last().unwrap(),
            CoordinatorAction::BroadcastClockStart { release_quantum } if *release_quantum == q + 1
        ));
    }
}

#[test]
fn test_early_registrations_across_two_links() {
    let mut state = CoordinatorState::new(create_config());

    state.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link0".into(),
    });
    state.apply(CoordinatorEvent::LinkRegister {
        node_id: 0,
        link_name: "link1".into(),
    });

    state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });

    let actions1 = state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
    assert_eq!(actions1.len(), 2);
    assert!(actions1.iter().all(|a| matches!(
        a,
        CoordinatorAction::SendLinkAck { node_id: 0, .. }
    )));

    state.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link0".into(),
    });
    let final_actions = state.apply(CoordinatorEvent::LinkRegister {
        node_id: 1,
        link_name: "link1".into(),
    });

    assert_eq!(final_actions.len(), 2);
    assert!(matches!(
        final_actions[1],
        CoordinatorAction::BroadcastClockStart { release_quantum: 0 }
    ));
}

#[test]
fn test_deferred_message_crosses_quantum_boundary() {
    let mut state = CoordinatorState::new(create_config());

    state.apply(CoordinatorEvent::NodeJoined { node_id: 0 });
    state.apply(CoordinatorEvent::NodeJoined { node_id: 1 });
    for n in 0..2 {
        state.apply(CoordinatorEvent::LinkRegister {
            node_id: n,
            link_name: "link0".into(),
        });
        state.apply(CoordinatorEvent::LinkRegister {
            node_id: n,
            link_name: "link1".into(),
        });
    }

    state.apply(CoordinatorEvent::SimulationMessage {
        src_node_id: 0,
        link_id: 10,
        delivery_vtime_ns: 15_000,
        sequence_number: 1,
        payload: vec![99],
    });

    state.apply(CoordinatorEvent::QuantumDone {
        node_id: 0,
        quantum: 0,
        vtime_ns: 10_000,
    });
    let q0_actions = state.apply(CoordinatorEvent::QuantumDone {
        node_id: 1,
        quantum: 0,
        vtime_ns: 10_000,
    });
    assert_eq!(q0_actions.len(), 1);
    assert!(matches!(
        q0_actions[0],
        CoordinatorAction::BroadcastClockStart { release_quantum: 1 }
    ));

    state.apply(CoordinatorEvent::QuantumDone {
        node_id: 0,
        quantum: 1,
        vtime_ns: 20_000,
    });
    let q1_actions = state.apply(CoordinatorEvent::QuantumDone {
        node_id: 1,
        quantum: 1,
        vtime_ns: 20_000,
    });
    assert_eq!(q1_actions.len(), 2);
    assert!(matches!(
        q1_actions[0],
        CoordinatorAction::RouteMessage {
            delivery_vtime_ns: 15_000,
            ..
        }
    ));
    assert!(matches!(
        q1_actions[1],
        CoordinatorAction::BroadcastClockStart { release_quantum: 2 }
    ));
}

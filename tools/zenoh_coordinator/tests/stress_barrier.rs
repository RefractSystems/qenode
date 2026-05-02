use std::sync::Arc;
use std::thread;
use zenoh_coordinator::barrier::{CoordMessage, QuantumBarrier};
use zenoh_coordinator::topology::Protocol;

#[test]
#[cfg_attr(miri, ignore)]
fn stress_barrier() {
    let n_nodes = 10;
    let rounds = 1000;
    let max_msgs = 1024;

    let barrier = Arc::new(QuantumBarrier::new(n_nodes, max_msgs));

    // QuantumBarrier::new() initializes current_quantum to 1, so quantum 0 is
    // treated as stale. Iterate 1..=rounds to match the barrier's quantum
    // numbering convention (also used by all unit tests in barrier.rs).
    for round in 1..=rounds {
        let mut handles = vec![];

        for node_id in 0..n_nodes {
            let barrier_clone = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let mut msgs = Vec::new();
                for m in 0..10 {
                    msgs.push(CoordMessage {
                        src_node_id: node_id.to_string(),
                        dst_node_id: ((node_id + 1) % n_nodes).to_string(),
                        base_topic: "test_topic".to_owned(),
                        delivery_vtime_ns: (10 - m) as u64, // out of order
                        sequence_number: m as u64,
                        protocol: Protocol::Uart,
                        payload: vec![m as u8],
                    });
                }
                barrier_clone
                    .submit_done(node_id.to_string(), round as u64, round as u64, msgs)
                    .unwrap()
            }));
        }

        let mut some_count = 0;
        let mut final_msgs = None;

        for handle in handles {
            if let Some(msgs) = handle.join().unwrap() {
                some_count += 1;
                final_msgs = Some(msgs);
            }
        }

        assert_eq!(
            some_count, 1,
            "Round {} must return exactly one Some",
            round
        );
        let final_msgs = final_msgs.unwrap();
        assert_eq!(
            final_msgs.len(),
            10 * n_nodes,
            "Must have exactly 100 messages"
        );

        // Verify ordering
        for i in 0..final_msgs.len() - 1 {
            assert!(
                final_msgs[i] <= final_msgs[i + 1],
                "Round {}: Messages not sorted at index {}: {:?} > {:?}",
                round,
                i,
                final_msgs[i],
                final_msgs[i + 1]
            );
        }

        barrier.reset();
    }
}

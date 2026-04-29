use std::sync::Arc;
use std::thread;
use zenoh_coordinator::barrier::{CoordMessage, QuantumBarrier};
use zenoh_coordinator::topology::Protocol;

fn dummy_msg(vtime: u64, seq: u64, src: &str) -> CoordMessage {
    CoordMessage {
        delivery_vtime_ns: vtime,
        src_node_id: src.to_owned(),
        dst_node_id: "dst".to_owned(),
        base_topic: "virtmcu/uart".to_owned(),
        sequence_number: seq,
        protocol: Protocol::Uart,
        payload: vec![],
    }
}

#[test]
fn stress_barrier_quantum_transitions() {
    let n_nodes = 4;
    let n_quanta = 100;
    let barrier = Arc::new(QuantumBarrier::new(n_nodes, 1024));

    let mut handles = vec![];

    for node_idx in 0..n_nodes {
        let b = Arc::clone(&barrier);
        let node_id = node_idx.to_string();

        let handle = thread::spawn(move || {
            for q in 1..=n_quanta {
                let msgs = vec![dummy_msg(q as u64, 0, &node_id)];

                loop {
                    let current = b.current_quantum();
                    if q as u64 > current + 1 {
                        thread::yield_now();
                        continue;
                    }

                    match b.submit_done(node_id.clone(), q as u64, current, msgs.clone()) {
                        Ok(Some(_all_msgs)) => {
                            break;
                        }
                        Ok(None) => {
                            while b.current_quantum() < q as u64 {
                                thread::yield_now();
                            }
                            break;
                        }
                        Err(e) => {
                            panic!("Node {} failed at quantum {}: {:?}", node_id, q, e);
                        }
                    }
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn stress_barrier_fast_node_overlap() {
    let n_nodes = 2;
    let n_quanta = 100;
    let barrier = Arc::new(QuantumBarrier::new(n_nodes, 1024));

    let b_node0 = Arc::clone(&barrier);
    let b_node1 = Arc::clone(&barrier);

    let h0 = thread::spawn(move || {
        for q in 1..=n_quanta {
            loop {
                let current = b_node0.current_quantum();
                if q as u64 > current + 1 {
                    thread::yield_now();
                    continue;
                }
                b_node0
                    .submit_done("0".to_owned(), q as u64, current, vec![])
                    .unwrap();
                break;
            }
        }
    });

    let h1 = thread::spawn(move || {
        for q in 1..=n_quanta {
            let current = b_node1.current_quantum();
            let _ = b_node1.submit_nowait("1".to_owned(), q as u64, current, vec![]);

            // Artificial delay to induce overlap
            if q % 10 == 0 {
                thread::yield_now();
            }
        }
    });

    h0.join().unwrap();
    h1.join().unwrap();
}

trait QuantumBarrierExt {
    fn submit_nowait(&self, node_id: String, q: u64, current: u64, msgs: Vec<CoordMessage>);
}

impl QuantumBarrierExt for QuantumBarrier {
    fn submit_nowait(&self, node_id: String, q: u64, current: u64, msgs: Vec<CoordMessage>) {
        loop {
            match self.submit_done(node_id.clone(), q, current, msgs.clone()) {
                Ok(_) => break,
                Err(_) => thread::yield_now(),
            }
        }
    }
}

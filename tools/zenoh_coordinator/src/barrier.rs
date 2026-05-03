use crate::topology::Protocol;
use core::cmp::Ordering;
use core::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordMessage {
    pub src_node_id: String,
    pub dst_node_id: String,
    pub base_topic: String,
    pub delivery_vtime_ns: u64,
    pub sequence_number: u64,
    pub protocol: Protocol,
    pub payload: Vec<u8>,
}

impl PartialOrd for CoordMessage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CoordMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        self.delivery_vtime_ns
            .cmp(&other.delivery_vtime_ns)
            .then_with(|| self.src_node_id.cmp(&other.src_node_id))
            .then_with(|| self.dst_node_id.cmp(&other.dst_node_id))
            .then_with(|| self.sequence_number.cmp(&other.sequence_number))
            .then_with(|| self.base_topic.cmp(&other.base_topic))
            .then_with(|| self.protocol.cmp(&other.protocol))
            .then_with(|| self.payload.cmp(&other.payload))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarrierError {
    Timeout,
    DuplicateDone,
    QuantumMismatch { expected: u64, got: u64 },
}

#[derive(Default)]
struct QuantumData {
    done_nodes: HashSet<String>,
    messages: Vec<CoordMessage>,
}

pub struct QuantumBarrier {
    n_nodes: usize,
    max_messages_per_node: usize,
    current_quantum: AtomicU64,
    state: Mutex<BarrierState>,
    all_done_cond: Condvar,
}

struct BarrierState {
    quanta: HashMap<u64, QuantumData>,
}

impl QuantumBarrier {
    pub fn new(n_nodes: usize, max_messages_per_node: usize) -> Self {
        Self {
            n_nodes,
            max_messages_per_node,
            current_quantum: AtomicU64::new(1),
            state: Mutex::new(BarrierState {
                quanta: HashMap::new(),
            }),
            all_done_cond: Condvar::new(),
        }
    }

    pub fn current_quantum(&self) -> u64 {
        self.current_quantum.load(AtomicOrdering::SeqCst)
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.quanta.clear();
    }

    pub fn submit_done(
        &self,
        node_id: String,
        quantum: u64,
        _expected_quantum: u64,
        mut messages: Vec<CoordMessage>,
    ) -> Result<Option<Vec<CoordMessage>>, BarrierError> {
        let mut state = self.state.lock().unwrap();
        let current = self.current_quantum.load(AtomicOrdering::SeqCst);

        if quantum < current {
            // Already finished this quantum; drop stale messages.
            return Ok(None);
        }

        // We allow arbitrarily large future quanta now

        let q_data = state.quanta.entry(quantum).or_default();
        if q_data.done_nodes.contains(&node_id) {
            return Err(BarrierError::DuplicateDone);
        }
        q_data.done_nodes.insert(node_id.clone());

        messages.sort();
        if messages.len() > self.max_messages_per_node {
            messages.truncate(self.max_messages_per_node);
        }
        q_data.messages.extend(messages);

        let mut current_q = self.current_quantum.load(AtomicOrdering::SeqCst);
        let mut all_sorted_msgs = Vec::new();
        let mut advanced = false;

        while let Some(current_q_data) = state.quanta.get(&current_q) {
            if current_q_data.done_nodes.len() == self.n_nodes {
                // We have a complete quantum!
                let mut data = state.quanta.remove(&current_q).unwrap();
                data.messages.sort();
                all_sorted_msgs.extend(data.messages);

                // Advance to next quantum
                current_q += 1;
                self.current_quantum
                    .store(current_q, AtomicOrdering::SeqCst);
                self.all_done_cond.notify_all();
                advanced = true;
            } else {
                // Current quantum is not complete
                break;
            }
        }

        // `Some(_)` signals barrier-satisfied (caller must publish the next START
        // and deliver any messages). The empty-vec case is meaningful: a quantum
        // can complete with zero in-flight messages (e.g. the kickstart DONE), and
        // collapsing it to `None` would silently swallow the advancement.
        if advanced {
            Ok(Some(all_sorted_msgs))
        } else {
            Ok(None)
        }
    }

    pub fn wait_for_all(&self, timeout: Duration) -> Result<Vec<CoordMessage>, BarrierError> {
        let state = self.state.lock().unwrap();
        let (_state, wait_result) = self.all_done_cond.wait_timeout(state, timeout).unwrap();

        if wait_result.timed_out() {
            Err(BarrierError::Timeout)
        } else {
            Ok(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_msg(vtime: u64, seq: u64, src: &str) -> CoordMessage {
        CoordMessage {
            delivery_vtime_ns: vtime,
            src_node_id: src.to_owned(),
            dst_node_id: "1".to_owned(),
            base_topic: "virtmcu/uart".to_owned(),
            sequence_number: seq,
            protocol: Protocol::Uart,
            payload: vec![],
        }
    }

    #[test]
    fn test_barrier_waits_for_all_3_nodes() {
        let barrier = QuantumBarrier::new(3, 1024);
        assert!(barrier
            .submit_done("0".to_owned(), 1, 1, vec![])
            .unwrap()
            .is_none());
        assert!(barrier
            .submit_done("1".to_owned(), 1, 1, vec![])
            .unwrap()
            .is_none());
        assert!(barrier
            .submit_done("2".to_owned(), 1, 1, vec![])
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_canonical_sort_same_vtime() {
        let barrier = QuantumBarrier::new(3, 1024);
        barrier
            .submit_done("2".to_owned(), 1, 1, vec![dummy_msg(10, 0, "2")])
            .unwrap();
        barrier
            .submit_done("0".to_owned(), 1, 1, vec![dummy_msg(10, 0, "0")])
            .unwrap();
        let sorted = barrier
            .submit_done("1".to_owned(), 1, 1, vec![dummy_msg(10, 0, "1")])
            .unwrap()
            .unwrap();

        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].src_node_id, "0");
        assert_eq!(sorted[1].src_node_id, "1");
        assert_eq!(sorted[2].src_node_id, "2");
    }

    #[test]
    fn test_canonical_sort_different_vtime() {
        let barrier = QuantumBarrier::new(3, 1024);
        barrier
            .submit_done("0".to_owned(), 1, 1, vec![dummy_msg(30, 0, "0")])
            .unwrap();
        barrier
            .submit_done("1".to_owned(), 1, 1, vec![dummy_msg(10, 0, "1")])
            .unwrap();
        let sorted = barrier
            .submit_done("2".to_owned(), 1, 1, vec![dummy_msg(20, 0, "2")])
            .unwrap()
            .unwrap();

        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].delivery_vtime_ns, 10);
        assert_eq!(sorted[1].delivery_vtime_ns, 20);
        assert_eq!(sorted[2].delivery_vtime_ns, 30);
    }

    #[test]
    fn test_barrier_reset_allows_next_quantum() {
        let barrier = QuantumBarrier::new(2, 1024);
        barrier.submit_done("0".to_owned(), 1, 1, vec![]).unwrap();
        barrier.submit_done("1".to_owned(), 1, 1, vec![]).unwrap();

        barrier.reset();

        assert!(barrier
            .submit_done("0".to_owned(), 2, 2, vec![])
            .unwrap()
            .is_none());
        assert!(barrier
            .submit_done("1".to_owned(), 2, 2, vec![])
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_barrier_duplicate_done_rejected() {
        let barrier = QuantumBarrier::new(2, 1024);
        barrier.submit_done("0".to_owned(), 1, 1, vec![]).unwrap();
        assert!(matches!(
            barrier.submit_done("0".to_owned(), 1, 1, vec![]),
            Err(BarrierError::DuplicateDone)
        ));
    }

    #[test]
    fn test_quantum_transition_race() {
        // This test simulates a node sending 'done' for the NEXT quantum
        // before the coordinator has explicitly called reset().
        // With auto-reset in submit_done, this should now pass.
        let barrier = QuantumBarrier::new(2, 1024);

        // Quantum 1
        barrier.submit_done("0".to_owned(), 1, 1, vec![]).unwrap();
        let res = barrier.submit_done("1".to_owned(), 1, 1, vec![]).unwrap();
        assert!(res.is_some()); // Quantum 1 finished

        // Node 0 is fast and sends 'done' for Quantum 2 immediately.
        // Even if the coordinator loop hasn't reached its own b.reset() call,
        // the barrier is already fresh.
        let res2 = barrier.submit_done("0".to_owned(), 2, 2, vec![]).unwrap();

        assert!(
            res2.is_none(),
            "Expected Ok(None) for the first node of the new quantum"
        );
    }

    #[test]
    fn test_admission_control_drops_excess() {
        let barrier = QuantumBarrier::new(1, 3);
        let msgs = vec![
            dummy_msg(0, 0, "0"),
            dummy_msg(0, 1, "0"),
            dummy_msg(0, 2, "0"),
            dummy_msg(0, 3, "0"),
            dummy_msg(0, 4, "0"),
        ];

        let result = barrier
            .submit_done("0".to_owned(), 1, 1, msgs)
            .unwrap()
            .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_admission_control_deterministic_truncation_payloads() {
        // Proves that messages with identical vtime/seq but different payloads
        // are still truncated deterministically regardless of input order.
        let max_msgs = 2;

        let mut m1 = dummy_msg(10, 1, "0");
        m1.payload = vec![1];
        let mut m2 = dummy_msg(10, 1, "0");
        m2.payload = vec![2];
        let mut m3 = dummy_msg(10, 1, "0");
        m3.payload = vec![3];

        // Different input permutations
        let perms = vec![
            vec![m1.clone(), m2.clone(), m3.clone()],
            vec![m3.clone(), m2.clone(), m1.clone()],
            vec![m2.clone(), m3.clone(), m1.clone()],
            vec![m1.clone(), m3.clone(), m2.clone()],
        ];

        let mut expected_result = None;
        for msgs in perms {
            let barrier = QuantumBarrier::new(1, max_msgs);
            let result = barrier
                .submit_done("0".to_owned(), 1, 1, msgs)
                .unwrap()
                .unwrap();
            assert_eq!(result.len(), 2);

            if let Some(expected) = &expected_result {
                assert_eq!(
                    &result, expected,
                    "Truncation was not deterministic across input permutations!"
                );
            } else {
                expected_result = Some(result);
            }
        }
    }

    #[test]
    fn test_admission_control_deterministic_truncation() {
        let barrier = QuantumBarrier::new(1, 3);
        let msgs = vec![
            dummy_msg(10, 4, "0"),
            dummy_msg(5, 1, "0"),
            dummy_msg(10, 3, "0"),
            dummy_msg(5, 2, "0"),
            dummy_msg(15, 5, "0"),
        ];

        let result = barrier
            .submit_done("0".to_owned(), 1, 1, msgs)
            .unwrap()
            .unwrap();

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].delivery_vtime_ns, 5);
        assert_eq!(result[0].sequence_number, 1);

        assert_eq!(result[1].delivery_vtime_ns, 5);
        assert_eq!(result[1].sequence_number, 2);

        assert_eq!(result[2].delivery_vtime_ns, 10);
        assert_eq!(result[2].sequence_number, 3);
    }

    #[test]
    fn test_admission_control_within_limit() {
        let barrier = QuantumBarrier::new(1, 3);
        let msgs = vec![
            dummy_msg(0, 0, "0"),
            dummy_msg(0, 1, "0"),
            dummy_msg(0, 2, "0"),
        ];

        let result = barrier
            .submit_done("0".to_owned(), 1, 1, msgs)
            .unwrap()
            .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_admission_control_zero_messages() {
        let barrier = QuantumBarrier::new(1, 3);
        let result = barrier
            .submit_done("0".to_owned(), 1, 1, vec![])
            .unwrap()
            .unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_admission_control_stress() {
        let max_msgs = 1024;
        let barrier = QuantumBarrier::new(1, max_msgs);

        let mut msgs = Vec::with_capacity(10_000);
        // Insert in reverse order to ensure worst-case sort complexity
        for i in (0..10_000).rev() {
            msgs.push(dummy_msg(i as u64, (10_000 - i) as u64, "0"));
        }

        let result = barrier
            .submit_done("0".to_owned(), 1, 1, msgs)
            .unwrap()
            .unwrap();

        assert_eq!(result.len(), max_msgs);
        assert_eq!(result[0].delivery_vtime_ns, 0);
        assert_eq!(result[1023].delivery_vtime_ns, 1023);
    }
}

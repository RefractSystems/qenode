use crate::topology::Protocol;
use core::cmp::Ordering;
use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use core::time::Duration;
use std::sync::{Condvar, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordMessage {
    pub src_node_id: u32,
    pub dst_node_id: u32,
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
            .then_with(|| self.protocol.cmp(&other.protocol))
            .then_with(|| self.payload.cmp(&other.payload))
    }
}

#[derive(Debug)]
pub enum BarrierError {
    Timeout,
    DuplicateDone,
    NodeIndexOutOfBounds(u32),
    QuantumMismatch { expected: u64, got: u64 },
}

pub struct QuantumBarrier {
    n_nodes: usize,
    max_messages_per_node: usize,
    done_count: AtomicUsize,
    message_buffer: Mutex<Vec<CoordMessage>>,
    all_done_cond: Condvar,
    done_nodes: Mutex<Vec<bool>>,
}

impl QuantumBarrier {
    pub fn new(n_nodes: usize, max_messages_per_node: usize) -> Self {
        Self {
            n_nodes,
            max_messages_per_node,
            done_count: AtomicUsize::new(0),
            message_buffer: Mutex::new(Vec::new()),
            all_done_cond: Condvar::new(),
            done_nodes: Mutex::new(vec![false; n_nodes]),
        }
    }

    pub fn submit_done(
        &self,
        node_id: u32,
        quantum: u64,
        expected_quantum: u64,
        mut messages: Vec<CoordMessage>,
    ) -> Result<Option<Vec<CoordMessage>>, BarrierError> {
        if quantum != expected_quantum {
            return Err(BarrierError::QuantumMismatch {
                expected: expected_quantum,
                got: quantum,
            });
        }

        let mut buffer = self
            .message_buffer
            .lock()
            .expect("message_buffer mutex poisoned");
        let mut done_nodes = self.done_nodes.lock().unwrap();

        if node_id as usize >= self.n_nodes {
            return Err(BarrierError::NodeIndexOutOfBounds(node_id));
        }
        if done_nodes[node_id as usize] {
            return Err(BarrierError::DuplicateDone);
        }
        done_nodes[node_id as usize] = true;

        // ARCH-5 Determinism Fix: We MUST sort before truncating.
        messages.sort();

        if messages.len() > self.max_messages_per_node {
            let excess = messages.len() - self.max_messages_per_node;
            tracing::warn!(
                "Node {} exceeded per-quantum message limit ({} > {}); dropping {} messages",
                node_id,
                messages.len(),
                self.max_messages_per_node,
                excess
            );
            messages.truncate(self.max_messages_per_node);
        }

        buffer.extend(messages);

        let count = self.done_count.fetch_add(1, AtomicOrdering::SeqCst) + 1;
        if count == self.n_nodes {
            let mut all_msgs = (*buffer).clone();
            all_msgs.sort();

            // ARCH-8: Auto-reset for next quantum to avoid race condition on CI.
            self.done_count.store(0, AtomicOrdering::SeqCst);
            buffer.clear();
            for d in done_nodes.iter_mut() {
                *d = false;
            }

            self.all_done_cond.notify_all();
            Ok(Some(all_msgs))
        } else {
            Ok(None)
        }
    }

    pub fn reset(&self) {
        let mut buffer = self
            .message_buffer
            .lock()
            .expect("message_buffer mutex poisoned");
        let mut done_nodes = self.done_nodes.lock().unwrap();
        self.done_count.store(0, AtomicOrdering::SeqCst);
        buffer.clear();
        for d in done_nodes.iter_mut() {
            *d = false;
        }
    }

    pub fn wait_for_all(&self, timeout: Duration) -> Result<Vec<CoordMessage>, BarrierError> {
        let buffer = self
            .message_buffer
            .lock()
            .expect("message_buffer mutex poisoned");
        if self.done_count.load(AtomicOrdering::SeqCst) == self.n_nodes {
            let mut msgs = buffer.clone();
            msgs.sort();
            return Ok(msgs);
        }

        let (buffer, wait_result) = self
            .all_done_cond
            .wait_timeout(buffer, timeout)
            .expect("all_done_cond wait failed");
        if wait_result.timed_out() {
            Err(BarrierError::Timeout)
        } else {
            let mut msgs = buffer.clone();
            msgs.sort();
            Ok(msgs)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_msg(vtime: u64, seq: u64, src: u32) -> CoordMessage {
        CoordMessage {
            delivery_vtime_ns: vtime,
            src_node_id: src,
            dst_node_id: 1,
            sequence_number: seq,
            protocol: Protocol::Uart,
            payload: vec![],
        }
    }

    #[test]
    fn test_barrier_waits_for_all_3_nodes() {
        let barrier = QuantumBarrier::new(3, 1024);
        assert!(barrier.submit_done(0, 0, 0, vec![]).unwrap().is_none());
        assert!(barrier.submit_done(1, 0, 0, vec![]).unwrap().is_none());
        assert!(barrier.submit_done(2, 0, 0, vec![]).unwrap().is_some());
    }

    #[test]
    fn test_canonical_sort_same_vtime() {
        let barrier = QuantumBarrier::new(3, 1024);
        barrier
            .submit_done(2, 0, 0, vec![dummy_msg(10, 0, 2)])
            .unwrap();
        barrier
            .submit_done(0, 0, 0, vec![dummy_msg(10, 0, 0)])
            .unwrap();
        let sorted = barrier
            .submit_done(1, 0, 0, vec![dummy_msg(10, 0, 1)])
            .unwrap()
            .unwrap();

        assert_eq!(sorted.len(), 3);
        assert_eq!(sorted[0].src_node_id, 0);
        assert_eq!(sorted[1].src_node_id, 1);
        assert_eq!(sorted[2].src_node_id, 2);
    }

    #[test]
    fn test_canonical_sort_different_vtime() {
        let barrier = QuantumBarrier::new(3, 1024);
        barrier
            .submit_done(0, 0, 0, vec![dummy_msg(30, 0, 0)])
            .unwrap();
        barrier
            .submit_done(1, 0, 0, vec![dummy_msg(10, 0, 1)])
            .unwrap();
        let sorted = barrier
            .submit_done(2, 0, 0, vec![dummy_msg(20, 0, 2)])
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
        barrier.submit_done(0, 0, 0, vec![]).unwrap();
        barrier.submit_done(1, 0, 0, vec![]).unwrap();

        barrier.reset();

        assert!(barrier.submit_done(0, 0, 0, vec![]).unwrap().is_none());
        assert!(barrier.submit_done(1, 0, 0, vec![]).unwrap().is_some());
    }

    #[test]
    fn test_barrier_duplicate_done_rejected() {
        let barrier = QuantumBarrier::new(2, 1024);
        barrier.submit_done(0, 0, 0, vec![]).unwrap();
        assert!(matches!(
            barrier.submit_done(0, 0, 0, vec![]),
            Err(BarrierError::DuplicateDone)
        ));
    }

    #[test]
    fn test_admission_control_drops_excess() {
        let barrier = QuantumBarrier::new(1, 3);
        let msgs = vec![
            dummy_msg(0, 0, 0),
            dummy_msg(0, 1, 0),
            dummy_msg(0, 2, 0),
            dummy_msg(0, 3, 0),
            dummy_msg(0, 4, 0),
        ];

        let result = barrier.submit_done(0, 0, 0, msgs).unwrap().unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_admission_control_deterministic_truncation_payloads() {
        // Proves that messages with identical vtime/seq but different payloads
        // are still truncated deterministically regardless of input order.
        let max_msgs = 2;

        let mut m1 = dummy_msg(10, 1, 0);
        m1.payload = vec![1];
        let mut m2 = dummy_msg(10, 1, 0);
        m2.payload = vec![2];
        let mut m3 = dummy_msg(10, 1, 0);
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
            let result = barrier.submit_done(0, 0, 0, msgs).unwrap().unwrap();
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
            dummy_msg(10, 4, 0),
            dummy_msg(5, 1, 0),
            dummy_msg(10, 3, 0),
            dummy_msg(5, 2, 0),
            dummy_msg(15, 5, 0),
        ];

        let result = barrier
            .submit_done(0, 0, 0, msgs)
            .unwrap()
            .unwrap_or_else(|| std::process::abort());

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
        let msgs = vec![dummy_msg(0, 0, 0), dummy_msg(0, 1, 0), dummy_msg(0, 2, 0)];

        let result = barrier
            .submit_done(0, 0, 0, msgs)
            .unwrap()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_admission_control_zero_messages() {
        let barrier = QuantumBarrier::new(1, 3);
        let result = barrier
            .submit_done(0, 0, 0, vec![])
            .unwrap()
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_admission_control_stress() {
        let max_msgs = 1024;
        let barrier = QuantumBarrier::new(1, max_msgs);

        let mut msgs = Vec::with_capacity(10_000);
        // Insert in reverse order to ensure worst-case sort complexity
        for i in (0..10_000).rev() {
            msgs.push(dummy_msg(i as u64, (10_000 - i) as u64, 0));
        }

        let result = barrier
            .submit_done(0, 0, 0, msgs)
            .unwrap()
            .unwrap_or_else(|| std::process::abort());

        assert_eq!(result.len(), max_msgs);
        assert_eq!(result[0].delivery_vtime_ns, 0);
        assert_eq!(result[1023].delivery_vtime_ns, 1023);
    }
}

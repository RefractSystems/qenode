use deterministic_coordinator::{
    barrier::{CoordMessage, QuantumBarrier},
    topology::Protocol,
};
use std::sync::Arc;
use tokio::task;

#[tokio::test]
async fn test_tx_done_ordering() {
    let barrier = Arc::new(QuantumBarrier::new(3, 100));

    let msg1 = CoordMessage {
        src_node_id: 0,
        dst_node_id: 1,
        delivery_vtime_ns: 50,
        sequence_number: 1,
        protocol: Protocol::Uart,
        payload: vec![1, 2, 3],
        base_topic: None,
    };

    let msg2 = CoordMessage {
        src_node_id: 0,
        dst_node_id: 2,
        delivery_vtime_ns: 100,
        sequence_number: 2,
        protocol: Protocol::Uart,
        payload: vec![4, 5, 6],
        base_topic: None,
    };

    let msg3 = CoordMessage {
        src_node_id: 1,
        dst_node_id: 0,
        delivery_vtime_ns: 75,
        sequence_number: 1,
        protocol: Protocol::Uart,
        payload: vec![7, 8, 9],
        base_topic: None,
    };

    let b_clone1 = barrier.clone();
    let b_clone2 = barrier.clone();
    let b_clone3 = barrier.clone();

    let t1 = task::spawn(async move { b_clone1.submit_done(0, 0, 0, vec![msg1, msg2]).unwrap() });

    let t2 = task::spawn(async move { b_clone2.submit_done(1, 0, 0, vec![msg3]).unwrap() });

    let t3 = task::spawn(async move { b_clone3.submit_done(2, 0, 0, vec![]).unwrap() });

    let res1 = t1.await.unwrap();
    let res2 = t2.await.unwrap();
    let res3 = t3.await.unwrap();

    let mut batch = None;
    if res1.is_some() {
        batch = res1;
    }
    if res2.is_some() {
        batch = res2;
    }
    if res3.is_some() {
        batch = res3;
    }

    assert!(batch.is_some());
    let batch = batch.unwrap();

    assert_eq!(batch.len(), 3);

    assert_eq!(batch[0].delivery_vtime_ns, 50); // From 0
    assert_eq!(batch[1].delivery_vtime_ns, 75); // From 1
    assert_eq!(batch[2].delivery_vtime_ns, 100); // From 0
}

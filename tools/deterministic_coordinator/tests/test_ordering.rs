use deterministic_coordinator::{
    barrier::{QuantumBarrier},
    topology::Protocol,
};
use std::sync::Arc;
use tokio::task;

#[tokio::test]
async fn test_tx_done_ordering() {
    let barrier = Arc::new(QuantumBarrier::new(3, 100));

  

    let b_clone1 = barrier.clone();
    let b_clone2 = barrier.clone();
    let b_clone3 = barrier.clone();

    let t1 = task::spawn(async move {
        b_clone1
            .submit_done(0, 0, 0, vec![msg1, msg2])
            .expect("test should succeed")
    });

    let t2 = task::spawn(async move {
        b_clone2
            .submit_done(1, 0, 0, vec![msg3])
            .expect("test should succeed")
    });

    let t3 = task::spawn(async move {
        b_clone3
            .submit_done(2, 0, 0, vec![])
            .expect("test should succeed")
    });

    let res1 = t1.await.expect("test should succeed");
    let res2 = t2.await.expect("test should succeed");
    let res3 = t3.await.expect("test should succeed");

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
    let batch = batch.expect("test should succeed");

    assert_eq!(batch.len(), 3);

    assert_eq!(batch[0].delivery_vtime_ns, 50); // From 0
    assert_eq!(batch[1].delivery_vtime_ns, 75); // From 1
    assert_eq!(batch[2].delivery_vtime_ns, 100); // From 0
}

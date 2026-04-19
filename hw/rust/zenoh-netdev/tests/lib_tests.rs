use std::collections::BinaryHeap;
use zenoh_netdev::OrderedPacket;

#[test]
fn test_ordered_packet_priority() {
    let mut heap = BinaryHeap::new();
    heap.push(OrderedPacket {
        vtime: 100,
        data: vec![1],
    });
    heap.push(OrderedPacket {
        vtime: 50,
        data: vec![2],
    });
    heap.push(OrderedPacket {
        vtime: 200,
        data: vec![3],
    });

    let first = heap.pop().unwrap();
    assert_eq!(
        first.vtime, 50,
        "Min-heap should pop the lowest vtime first"
    );

    let second = heap.pop().unwrap();
    assert_eq!(second.vtime, 100);
}

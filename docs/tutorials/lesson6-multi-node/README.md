# Lesson 6: Deterministic Multi-Node Networking

In this lesson, we explore how **virtmcu** handles multi-node coordination with absolute determinism. This replaces the traditional `WirelessMedium` typically found in Renode, allowing multiple independent QEMU instances to communicate reliably without losing deterministic execution.

## The Problem with Traditional Networking

Standard QEMU networking approaches like `-netdev socket,mcast=...` mixed with `-icount` do not provide deterministic delivery across multiple emulator instances. The UDP multicast is scheduled and delivered by the host Linux kernel asynchronously.

If two QEMU instances are running at different speeds (or if the host CPU context-switches them differently), packets arrive at different virtual times on different runs, completely breaking the reproducibility of multi-node tests.

## The Solution: Virtual Time-Stamped Frames

To guarantee determinism, virtmcu decouples **host delivery time** from **virtual arrival time**.

1. **TX (Transmission):** When a QEMU instance transmits a frame, the `netdev` backend intercepts it. It embeds QEMU's exact current virtual time (`QEMU_CLOCK_VIRTUAL`) into the packet header and publishes it to the Zenoh topic `sim/eth/frame/{node_id}/tx`.
2. **The Coordinator:** A lightweight Rust process (`tools/deterministic_coordinator`) subscribes to all `tx` topics. It acts as the physical medium (the air or wire). It applies propagation delay (and can apply attenuation/packet loss), updates the delivery virtual time in the header, and forwards the packet to other nodes on their `rx` topics.
3. **RX (Reception):** When QEMU receives a frame from Zenoh, it does **not** deliver it to the guest immediately. Instead, it extracts the `delivery_vtime_ns` and places the frame into a min-heap priority queue. A QEMU virtual timer is armed to fire exactly at `delivery_vtime_ns`.
4. **Delivery:** Only when QEMU's internal virtual clock reaches the exact designated nanosecond does the timer fire and inject the packet into the guest's NIC.

## Hands-On: Running the Coordinator

We have built a fast, asynchronous Rust coordinator.

You can compile and run it:
```bash
cd tools/deterministic_coordinator
cargo build --release
./target/release/deterministic_coordinator --delay-ns 5000000 # 5 ms propagation delay
```

## Running Multiple QEMU Instances

Start Node 1 (transmitting):
```bash
./scripts/run.sh --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    -netdev zenoh,node=1,id=n1 \
    -device virtio-net-device,netdev=n1 \
    -kernel firmware_node1.elf -nographic
```

Start Node 2 (receiving):
```bash
./scripts/run.sh --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb \
    -netdev zenoh,node=2,id=n2 \
    -device virtio-net-device,netdev=n2 \
    -kernel firmware_node2.elf -nographic
```

The coordinator will transparently bridge them. You can easily extend `tools/deterministic_coordinator/src/main.rs` to drop packets, implement distance-based attenuation logic, or simulate complex topologies.

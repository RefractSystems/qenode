# COORDINATOR-SYNC-PROTOCOL: TA/Coordinator Synchronization

## Status
**Draft** - April 2026

## Goal
To prevent a race condition where the `TimeAuthority` (TA) advances a node to quantum $Q+1$ before the `DeterministicCoordinator` has finished delivering inter-node messages from quantum $Q$. This ensures strict causal ordering: firmware in $Q+1$ MUST see all messages sent by peers in $Q$.

## The 8-Step Barrier Protocol

The `DeterministicCoordinator` acts as the gatekeeper for releasing the clock-advance reply back to the TA.

1.  **TA Advance**: TA sends `ClockAdvanceReq` to each node via its `ClockSyncTransport`.
2.  **Execution**: Each node executes quantum $Q$ (firmware runs).
3.  **Completion**: Each node sends all outbound inter-node messages to the coordinator, followed by a `done` signal to `sim/coord/{node_id}/done`.
4.  **Barrier Wait**: The coordinator waits until all $N$ nodes (as declared in the topology) have signalled `done` for the current quantum.
5.  **Delivery**: The coordinator sorts, masks (for partitioned networks), and delivers all buffered messages from quantum $Q$ to their respective destinations.
6.  **Release**: After all messages are delivered, the coordinator publishes a `start` signal to `sim/clock/start/{node_id}` for every node.
7.  **Clock Ack**: The `virtmcu-clock` device on each node receives the `start` signal and only THEN releases the `ClockReadyResp` back to the TA.
8.  **VTA Progression**: The TA receives all replies and proceeds to quantum $Q+1$.

## Implementation Details

### Node Side (Rust Plugin)
The `ZenohClockResponder::send_ready` method implements Step 7. If `coordinated=true` is set:
- It publishes the `done` signal.
- It blocks on a subscription to `sim/clock/start/{node_id}`.
- It only completes the Zenoh query (replying to TA) after the `start` signal arrives.

### Coordinator Side (Rust Tool)
The `DeterministicCoordinator` must track the `quantum_number` and ensure that the `start` signals are only emitted after all messages for that specific quantum have been dispatched.

## Failure Modes & Hardening

### Coordinator Deadlock
If a node crashes or QEMU is terminated, the coordinator might wait indefinitely for a `done` signal.
- **Solution**: The coordinator should monitor node liveliness. If a node disappears, the coordinator should either terminate the simulation or proceed by treating the missing node as having no messages.

### Standalone Mode
If `coordinated=false` (default), Step 3, 4, 5, 6, and 7 are bypassed. The node replies to the TA as soon as its own execution is complete. This is suitable for single-node tests or where inter-node causal ordering is not critical.

# Chapter 8: Parallel Discrete Event Simulation (PDES)

## 8.1 The Causality Problem in Distributed Simulation
When simulating a solitary microcontroller, temporal progression is trivial: the emulator's internal clock monotonically increments. However, the simulation of complex cyber-physical systems requires modeling multiple, geographically distributed MCUs interacting over shared networks (e.g., CAN buses, Ethernet, or wireless RF). 

Executing these virtual nodes as independent processes introduces a profound systemic vulnerability: **Non-Determinism**. The execution speed of an individual QEMU instance is heavily influenced by transient host OS scheduling, CPU cache contention, and inter-process communication latency. Without strict regulatory mechanisms, Node A might execute 100,000 instructions in the same wall-clock duration that Node B executes 50,000 instructions. 

If Node A transmits a network packet to Node B at Virtual Time $t=100$ns, but Node B has already "raced ahead" and processed its internal state up to $t=200$ns before the packet arrives, causality is irrevocably violated. The simulation is no longer a valid predictive model of physical reality.

## 8.2 Conservative Synchronization via Chandy-Misra-Bryant (CMB)
To enforce strict temporal coherence and global determinism, VirtMCU employs **Parallel Discrete Event Simulation (PDES)**. Specifically, it utilizes a "Conservative" synchronization model derived from the seminal Chandy-Misra-Bryant (CMB) algorithm.

In optimistic PDES models (like Time Warp), nodes execute aggressively; if a causality violation is detected, the system "rolls back" its state. Because rolling back the mutable state of a full QEMU virtual machine is computationally intractable, VirtMCU relies on the conservative approach: no node is permitted to advance its virtual clock unless it is mathematically guaranteed that it will not receive a message from the past.

### 8.2.1 The Quantum Synchronization Barrier
The conservative guarantee is implemented via discrete temporal windows known as **Quanta** (e.g., $1\mu s$ or $1ms$). 
1.  Every node in the simulation executes its firmware up to the boundary of Quantum $Q$.
2.  Upon reaching the boundary, the node suspends its virtual CPU and transmits a synchronization message to a central **Deterministic Coordinator**.
3.  The Coordinator acts as an absolute execution barrier. It accumulates these signals until it receives confirmation from *every* node in the topological graph that Quantum $Q$ is complete.
4.  Once the barrier is fulfilled, the Coordinator evaluates all network messages generated during Quantum $Q$, distributes them to their respective destination nodes, and subsequently broadcasts the authorization to advance to Quantum $Q+1$.

## 8.3 Canonical Event Ordering and Tie-Breaking
In a distributed system, it is inevitable that two discrete events—such as two nodes transmitting packets simultaneously—will claim the identical virtual timestamp. If the delivery order of these concurrent events depends on unpredictable host networking latency, the simulation remains non-deterministic.

VirtMCU resolves this ambiguity through a rigorous, **Canonical Tie-Breaking** protocol. The Deterministic Coordinator queues all messages and sorts them explicitly before delivery based on three immutable criteria:
1.  **Virtual Timestamp ($vtime$):** The primary sorting key.
2.  **Source Node Identifier:** If timestamps are identical, the statically assigned topological Node ID dictates precedence.
3.  **Sequence Number:** If a single node emits multiple messages at the exact same nanosecond, a monotonically incrementing counter resolves the final order.

By imposing this mathematical total ordering, VirtMCU guarantees that same inputs, combined with the same topology and global random seed, will yield bit-identical simulation results on any host hardware.

## 8.4 Summary
Parallel Discrete Event Simulation provides the theoretical scaffolding required to maintain causal integrity in distributed emulations. By enforcing a conservative quantum barrier and applying strict canonical sorting to network events, VirtMCU eliminates timing jitter and achieves the "Gold Standard" of emulation: absolute global determinism.

## 8.5 Exercises
1.  **The Lookahead Dilemma:** The performance of a conservative PDES system is highly dependent on the "Lookahead" value (the size of the quantum). Discuss the architectural trade-offs between a $1\mu s$ quantum and a $1ms$ quantum. Specifically, analyze the impact on network throughput (IPC overhead) versus the latency fidelity of an emulated CAN bus.
2.  **Optimistic vs. Conservative Realities:** Research the "Time Warp" mechanism used in Optimistic PDES. Why is taking a "snapshot" of a QEMU process for potential rollback fundamentally more difficult than taking a snapshot of a pure, stateless mathematical simulation model?

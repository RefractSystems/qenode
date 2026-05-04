# Infrastructure Tests

This directory contains integration tests whose System-Under-Test (SUT) is the **VirtMCU framework itself**, specifically the `deterministic_coordinator` binary, the PDES (Parallel Discrete Event Simulation) engine, and the underlying Zenoh topology constraints.

## Transport Direct-Access Exception

Unlike tests in `tests/integration/simulation/`, these tests are explicitly allowed to interact with Zenoh.

To validate the `zenoh_coordinator` or peer-to-peer determinism, these tests must manually spoof QEMU nodes, intercept Zenoh clock advance messages, or validate dropped packets directly on the Zenoh router.

**Rule**: Any file in this directory that imports Zenoh directly MUST declare its exception reason at the very top of the file:
```python
# ZENOH_HACK_EXCEPTION: Tests zenoh_coordinator natively by mocking QEMU nodes
```

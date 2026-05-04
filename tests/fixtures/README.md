# Test Fixtures

This directory contains resources, topologies, and guest applications used by the integration test suites.

## Scope & Purpose

- **`guest_apps/`**: Source code (C/Assembly/Rust/Device Trees) for minimal firmware payloads used to test specific peripherals or behaviors. These are built dynamically during test execution (or cached via the `Makefile` pipelines) to inject into `simulation.add_node()`.
- **`topologies/`**: YAML representations of multi-node network topologies, used by the `DeterministicCoordinator` and test runners to validate complex network configurations (e.g. LIN, FlexRay, Zenoh router routing).

These are not tests themselves, but the foundational data that drives the black-box simulation testing.

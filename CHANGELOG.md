# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- ARCH-21: CoSimBridge formalization (Milestone 27.1).
- Hybrid Replay Architecture (ADR-017).

## [0.9.10] - 2026-05-13
### Added
- Dedicated `InsnTrace` FlatBuffer protocol for high-fidelity TCG instruction tracing.
- `federation-id` support for multi-instance traceability and isolation.
- Coordinator node-join validation to prevent misconfigured topologies.
- Canonical topic functions for actuator and sensor namespaces in `virtmcu-api`.

### Changed
- Renamed `time-authority` to `physical-node` for architectural clarity.
- Migrated physics gateway to a decoupled protocol as part of Phase X.
- Refactored `rf802154` to use FlatBuffers for structured frame parsing.

### API Stability

| API surface                         | Status      | Notes |
|-------------------------------------|-------------|-------|
| ClockAdvanceReq wire format         | Frozen      | Breaking changes require major version bump |
| DataTransport trait                 | Stable      | Additive changes only |
| topics::sim_topic::* functions      | Stable      | |
| PhysicsTrigger / PhysicsDone FBS    | Stable      | |
| InsnTrace FBS (new in this pass)    | Beta        | Field set may grow |
| TraceEvent FBS                      | Stable      | |
| VirtmcuTestEnv builder API          | Stable      | |
| World YAML topology schema          | Stable      | |
| SHM futex protocol (cyber_bridge)   | Beta        | Linux-only; cross-platform pending |
| Zenoh topic namespace               | Stable      | |

### Migration
No breaking changes from 0.9.9.

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Integrated physics-in-the-loop testing for pendulum controller (ARCH-21).
- Deterministic RESD sensor fixture generator for closed-loop validation.
- Rust implementation of pendulum mock-physics to replace Python spin-poll (Milestone 24.1).

## [0.9.10] - 2026-05-13

### Added
- Dedicated `InsnTrace` FlatBuffer schema for high-fidelity instruction tracing.
- Canonical topic management in `virtmcu-api` for actuators, sensors, and telemetry.
- Node-join validation in `DeterministicCoordinator` to prevent topology mismatches.
- Federation-id support for multi-instance simulation traceability.

### Changed
- Renamed `time-authority` to `physical-node` to better reflect its role in the CPS stack.
- Updated TCG tracer to use the new `InsnTrace` protocol instead of overloading `TraceEvent`.
- Migrated inter-node communication protocols to formalized FlatBuffer schemas.

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

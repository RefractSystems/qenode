# Unit Tests

This directory contains fast, isolated white-box and Python logic tests.

## Scope & Purpose

- **Target:** Python utilities, parsers, schema generators, QMP wrappers, MCP servers, and tooling scripts.
- **Rules:** 
  - **No QEMU:** These tests must run completely isolated from the emulator.
  - **No Networks:** Do not spawn real network routers or background daemons. Mock all dependencies where necessary.
  - **Speed:** Execution must be near-instant.

These tests correspond to the "White-Box Internals" and tooling layers of our Bifurcated Testing Strategy.

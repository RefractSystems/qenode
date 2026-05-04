# Firmware

This directory contains pre-compiled, "golden" firmware binaries used for validation and binary fidelity testing.

## Scope & Purpose

- **Target:** Vendor SDK examples, Zephyr/NuttX builds, and compiled reference images that are too complex to build dynamically within the standard pytest loop.
- **Rules:**
  - Used strictly for read-only validation against the emulator.
  - Binaries stored here MUST comply with the project mandates (e.g., proper tracking via `PROVENANCE.md` or checksum validation).

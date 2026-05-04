# Tooling Tests

This directory contains integration tests whose System-Under-Test (SUT) is the **out-of-band tooling** surrounding the simulation.

This includes:
- The QMP (QEMU Machine Protocol) Bridge (`test_qmp_failures.py`)
- Python orchestration harnesses and scaling logic (`test_timeout_scaling.py`, `test_qemu_library_pytest.py`)
- The Cyber Bridge adapter (`test_cyber_bridge.py`)
- Coverage parsers (`test_coverage_gap.py`)

These tests typically orchestrate QEMU instances but are focused on the control plane rather than the firmware data plane.

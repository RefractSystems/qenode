# Handoff: Phase 11.4 — FirmwareStudio Upstream Migration

This document serves as the technical handoff from the `virtmcu` low-level repository to the `FirmwareStudio` high-level platform repository. It contains the instructions, architectural contracts, and execution steps for the new AI instance to complete Phase 11.4.

---

## 🤖 Message to the New Gemini Agent

Welcome to the **FirmwareStudio** migration. You are inheriting a platform that currently uses a legacy "Python-in-the-loop" simulation model. Your goal is to migrate it to use the new **virtmcu** framework.

**Your prime objective is to delete Python from the simulation hot-loop.**

The `virtmcu` project has successfully implemented native QEMU C/Rust plugins for time synchronization, networking, and co-simulation. You must now refactor the parent `FirmwareStudio` project to adopt these native capabilities.

---

## 🏗️ Technical State of `virtmcu` (The API Contract)

You are interfacing with a QEMU 11.0.0-rc3 based framework with the following capabilities:

1.  **Container Image:** Use `ghcr.io/refractsystems/virtmcu:latest`. It contains the patched QEMU binaries and all `.so` dynamic plugins.
2.  **Clock Master:** QEMU is now a **Time Slave**. The physics engine (MuJoCo/OpenUSD) MUST act as the **Time Master** (TimeAuthority).
    *   **Zenoh Topic:** `sim/clock/advance/{node_id}`
    *   **Payload:** `{ uint64 delta_ns; uint64 mujoco_time_ns; }`
    *   **Logic:** QEMU blocks at translation block boundaries until it receives this message.
3.  **Machine Generation:** Do not hardcode QEMU CLI arguments. Use the `virtmcu` translation pipeline:
    *   **Command:** `run.sh --repl myboard.repl --kernel firmware.elf` (or `--yaml`)
    *   **Architecture:** Supported architectures include `arm` and `riscv64` (automatically detected).
4.  **Co-Simulation:**
    *   **Path A:** MMIO over Unix socket (`mmio-socket-bridge`).
    *   **Path B:** Industry-standard AMD/Xilinx Remote Port over Unix socket (`remote-port-bridge`).

---

## 📋 Phase 11.4 Execution Plan

Execute the following steps within the `FirmwareStudio` repository:

### Step 1: Infrastructure & Dependency Update
*   [ ] **Clean up QEMU legacy:** Locate and remove any scripts that download or install standalone QEMU 10.2.1.
*   [ ] **Update Orchestration:** Modify `docker-compose.yml` or the platform's node manager to pull the `virtmcu` container image.
*   [ ] **Mount Hardware:** Ensure `.repl` or `.yaml` hardware descriptions are mounted into the simulation nodes.

### Step 2: Delete "Python-in-the-Loop" Agents
*   [ ] **Remove `node_agent.py`:** This script previously mediated between physics and QMP. It is now obsolete.
*   [ ] **Remove `shm_bridge.py`:** Native Zenoh networking and `mmio-socket-bridge` replace this legacy shared-memory hack.
*   [ ] **Cleanup RPCs:** Remove any internal gRPC or Socket.io logic that existed solely to talk to these deleted Python agents.

### Step 3: Implement the Physics TimeAuthority
*   [ ] **Refactor Physics Loop:** Update the main physics tick function (where `mj_step()` is called).
*   [ ] **Add Zenoh Clock Advance:** After each physics step, send the Zenoh `GET` request to `sim/clock/advance/{node_id}`.
*   [ ] **Enforce Lockstep:** Ensure the physics engine blocks until the simulation nodes acknowledge the time advance.

### Step 4: Machine Definition Refactoring
*   [ ] **Adopt REPL/YAML:** Transition the UI and the "Create Board" logic to emit standard Renode `.repl` or OpenUSD-aligned `.yaml` files.
*   [ ] **Launch via `run.sh`:** Change the node startup command to use the unified `virtmcu` `run.sh` wrapper, which handles DTB generation and plugin loading automatically.

### Step 5: I/O Abstraction (SAL/AAL) Transition
*   [ ] **Decommission IVSHMEM:** Remove the hardcoded PCI IVSHMEM device from the simulated ARM machine.
*   [ ] **Native Peripheral Mapping:** Map sensor data (IMU, LIDAR) and actuator commands (Motors) to native `virtmcu` QOM devices or Zenoh topics defined in the new SAL/AAL spec.

---

## 🛠️ Recommended Devcontainer Setup

For the `FirmwareStudio` repo, use a devcontainer configured as follows:
- **Base:** `mcr.microsoft.com/devcontainers/python:3.12`
- **Features:** 
    - `docker-in-docker` (to launch `virtmcu` simulation nodes).
- **PostCreateCommand:**
    - Install `zenoh-python`.
    - Mirror the global memory persistence logic from `virtmcu/.devcontainer/devcontainer.json`.

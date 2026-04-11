# FirmwareStudio Migration Guide

This guide details the steps required to migrate the FirmwareStudio proof-of-concept (POC) architecture to fully integrate with `virtmcu`'s Phase 7+ capabilities.

## Phase 7: Eliminating Python from the Simulation Loop

In the original FirmwareStudio POC, the cyber-node architecture relied heavily on a Python `node_agent.py` running in the `cyber/src/` directory. This script managed the Zenoh connection and synchronized QEMU's clock via a UNIX socket.

With virtmcu Phase 7, **Python is strictly banned from QEMU's execution runtime**. The `node_agent.py` has been entirely replaced by a native C QOM plugin (`hw/zenoh/zenoh-clock.c`).

### 1. Delete `cyber/src/node_agent.py`
The Python agent is obsolete. Remove the `cyber/` directory entirely from the FirmwareStudio repository. The cyber-node container now directly executes QEMU.

### 2. Update `docker-compose.yml` and `worlds/*.yml`
The `docker-compose.yml` for the cyber-node must be updated to launch QEMU directly using `virtmcu`'s patched binary.

**Old POC Design (in FirmwareStudio):**
```yaml
cyber-node:
  build: ./cyber
  environment:
    QEMU_CLOCK_SOCKET: /tmp/qemu-clock.sock
    CLOCK_MODE: slaved
  command: ["python3", "src/node_agent.py"]
```

**New virtmcu Design:**
```yaml
cyber-node:
  image: virtmcu:latest  # Or build from the virtmcu repository
  environment:
    ZENOH_ROUTER: tcp/zenoh-router:7447
  command: [
    "/app/scripts/run.sh",
    "--yaml", "/app/boards/flight_controller.yaml",
    "-kernel", "/app/firmware/fw.elf",
    "-device", "zenoh-clock,mode=suspend,node=0"
  ]
```

### 3. Replace Hardcoded Machines with YAML/Device Tree
The original POC used a hardcoded Cortex-A15 PCI machine with IVSHMEM for sensors. virtmcu uses `arm-generic-fdt` and the modern YAML hardware description (introduced in Phase 3.5).

*   **Action:** Write a `.yaml` board description (or migrate a Renode `.repl` using `tools/repl2yaml.py`) for your cyber-nodes.
*   **Action:** Pass this YAML to `scripts/run.sh --yaml <file>` instead of relying on C machine structs.

### 4. Switch from `-icount` to `slaved-suspend`
The POC used QEMU's `-icount` mode exclusively. virtmcu introduces `slaved-suspend` mode (`-device zenoh-clock,mode=suspend`), which runs at ~95% free-run speed.
*   **Action:** Use `mode=suspend` for any firmware that does not strictly measure sub-quantum intervals (e.g., standard control loops). Keep `mode=icount` only for firmware doing high-precision sub-quantum timer polling.

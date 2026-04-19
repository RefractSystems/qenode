import re

with open("PLAN.md", "r") as f:
    content = f.read()

new_phase_21 = """## Phase 21 — High-Throughput WiFi Simulation (802.11 over Zenoh)

**Goal**: Support WiFi (802.11) as a high-throughput, deterministic communication channel, enabling complex mobile and infrastructure-based simulation scenarios. In strict accordance with ADR-006 (Binary Fidelity), we will not invent a "generic virtmcu WiFi interface". Instead, we will emulate specific, real-world hardware interfaces to run unmodified vendor firmware.

**Tasks**:
- [x] **21.1** **Zenoh-WiFi Header & Protocol**: Define a FlatBuffers schema (`wifi_generated.rs`) in `virtmcu-api` containing `delivery_vtime_ns`, `size`, `channel`, `rssi`, `snr`, and `frame_type` (Management, Control, Data), fulfilling the schema evolution requirements of Phase 14.8.
- [ ] **21.2** **Option A (Initial Target): SPI/UART WiFi Co-Processor (e.g. ATWINC1500 or ESP32 AT-Command)**: 
  - **Prerequisite 1:** Add safe Rust bindings to `virtmcu-qom` for QEMU's `SSISlave` and `SerialDevice` (or `CharBackend`), enabling dynamic QOM plugins to act as SPI slaves or UART-attached coprocessors.
  - **Prerequisite 2:** Extend `arm-generic-fdt` and `yaml2qemu.py` to support instantiating and wiring SPI controllers (e.g., PL022) and their child devices.
  - **Prerequisite 3:** Write and verify a simple "SPI Echo" and "UART Echo" bare-metal firmware against dummy Rust devices to prove the bus plumbing works deterministically.
  - Source a validated Zephyr ELF (e.g., `wifi_dhcpv4`) for a specific board (e.g., an STM32 with SPI WiFi).
  - Implement the specific SPI slave or UART AT-command behavior expected by the firmware's host driver.
  - The `zenoh-wifi` backend translates these bus commands into Zenoh FlatBuffers.
- [ ] **21.3** **Option B (Secondary Target): VirtIO-WLAN for Linux**: 
  - For high-throughput Linux-based tests (Cortex-A15/RISC-V), implement the `virtio-wlan` specification.
  - Validate with a standard Linux kernel and Buildroot rootfs using standard `mac80211` VirtIO drivers.
- [ ] **21.4** **Option C (Long-Term/Stretch): Integrated Silicon (ESP32-C3)**: 
  - Extremely complex due to undocumented MAC registers and binary blobs. Deferred until the SPI and VirtIO paths are robust.
- [ ] **21.5** **High-Throughput Buffer Management**: Optimized RX/TX rings in Rust to handle standard 802.11 MTU (2304 bytes MSDU) without impacting simulation quantum latency."""

content = re.sub(r'## Phase 21 — High-Throughput WiFi Simulation \(802\.11 over Zenoh\).*?(?=### Phase 21 Outcomes, Testing, and Hardening)', new_phase_21 + '\n\n', content, flags=re.DOTALL)

with open("PLAN.md", "w") as f:
    f.write(content)

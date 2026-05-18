# Quarantined Integration Tests

These tests were quarantined on 2026-05-18 during the reference peripheral
stabilization phase (RFC-0042 Stage 2).

## Re-entry criteria

A test moves back to `tests/native_integration/tests/` only after:

1. The peripheral it tests has been ported to the RFC-0042 API
   (`register_link` / `reserve_link` / `VtimeIngress::new_for_link`).
2. `make test-reference-peripheral` passes clean.
3. The test itself passes with both `unix` and `zenoh` transports.
4. `make test-lint` is clean.

## Current quarantine list

| File | Peripheral / domain | Re-entry blocker |
|------|---------------------|-----------------|
| canfd.rs | CAN-FD bus | RFC-0042 port pending |
| flexray_bridge.rs | FlexRay bus | RFC-0042 port pending |
| lin_bridge.rs | LIN bus | RFC-0042 port pending |
| spi_bridge.rs | SPI bus | RFC-0042 port pending |
| actuator.rs | Physics actuator | RFC-0042 port pending |
| uart.rs | UART chardev | RFC-0042 port pending |
| boot_arm.rs | ARM boot smoke | unblocked but low priority |
| bql_starvation.rs | BQL stress | unblocked but low priority |
| clock_suspend.rs | Clock sync | unblocked but low priority |
| coordinator_stress.rs | Coordinator stress | unblocked but low priority |
| complex_board.rs | Multi-peripheral | depends on all ports |
| dummy_network_bad_topology.rs | Error path | unblocked but low priority |
| federation_id.rs | Coordinator federation | unblocked but low priority |
| firmware_golden.rs | Firmware hash | unblocked but low priority |
| irq_stress.rs | IRQ stress | unblocked but low priority |
| jitter.rs | Clock jitter | unblocked but low priority |
| mac_parsing.rs | Ethernet MAC | RFC-0042 port pending |
| pendulum_boot.rs | Pendulum demo | unblocked but low priority |
| pendulum_e2e_compose.rs | Pendulum demo | unblocked but low priority |
| pendulum_loop.rs | Pendulum demo | unblocked but low priority |
| pendulum_smoke.rs | Pendulum demo | unblocked but low priority |
| plugin_multiplexing.rs | DSO loading | unblocked but low priority |
| qmp.rs | QMP protocol | unblocked but low priority |
| qmp_failures.rs | QMP error paths | unblocked but low priority |
| reconnect.rs | Transport reconnect | unblocked but low priority |
| riscv_complex.rs | RISC-V board | unblocked but low priority |
| svd2header.rs | SVD tooling | unblocked but low priority |
| svd_hash_handshake.rs | SVD hash | unblocked but low priority |
| svd_patch.rs | SVD tooling | unblocked but low priority |
| tcg_tracer.rs | TCG plugin | unblocked but low priority |
| telemetry_throughput.rs | Telemetry | unblocked but low priority |
| telemetry_wfi.rs | Telemetry | unblocked but low priority |
| yaml_boot_advanced.rs | YAML boot | unblocked but low priority |

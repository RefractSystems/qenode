# Debugging Playbook

This playbook provides a quick reference for diagnosing common VirtMCU failures based on their symptoms.

## Symptom-Driven Decision Tree

### QEMU exits with `rc=-11` (SIGSEGV)
*   **Check ASan:** Look for `AddressSanitizer` in the logs.
*   **Check Module Loading:** Did QEMU fail to find a plugin `.so`? Check `modinfo` and `QEMU_MODULE_DIR`.
*   **Check Symbol Exports:** Run `nm -D` on the `.so`. Is `qemu_module_dummy` present?
*   **Case Study:** See [Postmortem: FlexRay SIGSEGV](../postmortem/2026-05-01-flexray-rc-11-segfault.md).

### "Stall Detected" in Logs
*   **Check Multiplier:** Is the host under high load? Increase the timeout multiplier.
*   **Check MMIO Blocking:** Is a peripheral plugin blocking the vCPU thread for too long?
*   **Check BQL:** Is a background thread holding the BQL and never releasing it?

### Test passes locally, fails in CI
*   **Check Cargo Isolation:** Are you sharing a `target/` directory between host and container?
*   **Check Port Contention:** Are you using a hardcoded port that is already in use on the CI runner?
*   **Check Stale Artifacts:** Run `make clean-sim` to ensure no old `.so` files are polluting the loader.

### Frame published but not received
*   **Check Topic Convention:** Does the sender use `{topic}/{node}/tx` and the receiver `{topic}/{node}/rx`?
*   **Check Zenoh Connectivity:** Is the Zenoh router running? Use `zenoh-probe` to see active sessions.

## The "Fail Loudly" Checklist
If a simulation is behaving incorrectly but not crashing:
1.  **Enable Logging:** Run with `sim_info!` and `sim_warn!` macros enabled.
2.  **Verify DTB:** Run `fdtdump` on the generated DTB. Is the peripheral mapped at the correct address?
3.  **Trace MMIO:** Use QEMU's `-d guest_errors,unimp,trace:memory_region_ops_read,trace:memory_region_ops_write` to see every guest access.

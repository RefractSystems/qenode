# Part VIII: War Stories (Case Studies)

Practical engineering is often learned through failure. This section documents real-world bugs encountered during the development of VirtMCU, presented as pedagogical case studies.

## Case Studies

1.  **[The FlexRay SIGSEGV (2026-05-01)](2026-05-01-flexray-rc-11-segfault.md)**: A lesson in cascading silent failures and the importance of name alignment.
2.  **[QEMU Plugin Visibility (2026-04-29)](2026-04-29-qemu-plugin-visibility.md)**: Why exported symbols matter for dynamic loading.
3.  **[CI ASan Failures (2026-04-21)](2026-04-21-ci-asan-failures.md)**: Managing timeouts and resource contention in a slow CI environment.
4.  **[ARM Generic FDT ASan Crash](POSTMORTEM-arm-generic-fdt-asan-crash.md)**: A deep dive into memory corruption in the FDT loader.

## Why read these?
Each case study illustrates a failure mode that is not obvious from the architecture documentation alone. They bridge the gap between "how it works" and "how it breaks."

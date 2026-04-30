# The Virtmcu Engineer's Manual: Development, Testing, and Infrastructure

## Preface

Welcome to the **Virtmcu Engineer's Manual**. While the *Architecture Specification* (located in `/docs/architecture/`) explains how the system is designed, this manual explains how to **build, test, and maintain** it.

VirtMCU is a complex, multi-language project that bridges the gap between C (QEMU), Rust (Peripherals), Python (Orchestration), and DevOps (Docker, CI/CD). To maintain the high standards of an enterprise-grade digital twin platform, every engineer must adhere to the workflows, safety guardrails, and testing standards defined in these chapters.

---

## Table of Contents

### Part I: The Environment
- **[Chapter 1: The Build System](01-build-system.md)**: Understanding `meson`, `cargo`, and the bifurcated QEMU/Rust build process.
- **[Chapter 2: Containerized Development](02-containerized-development.md)**: Working within the DevContainer and managing the development lifecycle.

### Part II: Quality Assurance
- **[Chapter 3: Testing Strategy & Guidelines](03-testing-strategy.md)**: From unit tests to multi-node Robot Framework integration suites.
- **[Chapter 4: Continuous Integration & Delivery](04-continuous-integration.md)**: Our GitHub Actions pipeline, ASan tiers, and release management.

### Part III: Evolution
- **[Chapter 5: Project History & Milestones](05-project-history.md)**: A record of completed phases and the technical evolution of the project.

---

## The Engineering Philosophy

### 1. Automation Over Intuition
If a check can be automated (lint, FFI export verify, address alignment), it must be in the `Makefile` and enforced in CI. We do not rely on developer memory.

### 2. Hermeticity
Our build environment is strictly containerized. "It works on my machine" is solved by "It works in the DevContainer." All production artifacts are built in the `builder` image.

### 3. Test-First Evolution
No feature is complete without a corresponding test in the integration suite. For bug fixes, an empirical reproduction script is a prerequisite for a pull request.

### 4. Zero-Warning Policy
We maintain a zero-warning threshold for `clippy`, `ruff`, `mypy`, and `cppcheck`. Technical debt is a liability that we aggressively manage.

# Part I: Foundations

## Welcome to VirtMCU

Welcome to the **VirtMCU** curriculum. Before we dive into the complex internals of virtual time synchronization and hardware emulation, we must first understand the "Why" and the "How" of the system's development. This section provides the historical context and the practical tools you need to begin your journey.

VirtMCU is a complex, multi-language project that bridges the gap between C (QEMU), Rust (Peripherals), Python (Orchestration), and DevOps (Docker, CI/CD). To master this system, you must first master the environment in which it lives.

---

## Table of Contents

### 1. [The VirtMCU Story](05-project-history.md)
A record of the technical evolution of the project, from its inception to its current state as a hardened, deterministic emulator. Understanding the history helps clarify the "why" behind many architectural decisions.

### 2. [Laboratory Setup](02-containerized-development.md)
Setting up your development environment. We leverage containerization to ensure that "it works on my machine" is a guarantee, not a gamble.

### 3. [The Build System](01-build-system.md)
Understanding `meson`, `cargo`, and the bifurcated QEMU/Rust build process. Here, you will learn how the "Forge" works—how source code is transformed into a high-performance simulation engine.

---

## The Engineering Philosophy

### 1. Automation Over Intuition
If a check can be automated (lint, FFI export verify, address alignment), it must be in the `Makefile` and enforced in CI. We do not rely on developer memory.

### 2. Hermeticity
Our build environment is strictly containerized. The DevContainer is your standardized laboratory.

### 3. Test-First Evolution
No feature is complete without a corresponding test. We build with the confidence that every change is verified against a rigorous suite of deterministic tests.

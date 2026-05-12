# Prerequisites

Welcome to the Graduate Course on VirtMCU. This course is designed for students with a strong background in software engineering who wish to master the art of deterministic multi-node firmware simulation. Before we dive into the architecture of VirtMCU, we must ensure a common baseline of technical proficiency.

## 0.1 Required Background

This course does not require extensive prior experience in embedded systems—we will teach you the silicon-side fundamentals. However, it *does* require mastery of modern software engineering tools and systems programming.

### C Programming
VirtMCU and QEMU are built on C. You must be comfortable with pointers, manual memory management, and C-style object orientation (structs with function pointers).
*   **Canonical Reference:** *The C Programming Language (2nd Edition)* by Kernighan and Ritchie (K&R). Specifically, Chapters 5 (Pointers and Arrays) and 6 (Structures).

### Rust Programming
The core of the VirtMCU peripheral subsystem is migrating to Rust. You will be writing and debugging Rust code.
*   **Canonical Reference:** *The Rust Programming Language* (The Book). You must have a deep understanding of Chapters 1–10 (Ownership, Borrowing, Enums, and Generics).

### Systems Programming & OS Concepts
You must understand what a process, a thread, and a mutex are. VirtMCU relies heavily on multi-threaded execution and synchronization.
*   **Canonical Reference:** *Operating Systems: Three Easy Pieces* (OSTEP) by Arpaci-Dusseau. Read §1–§5 (Virtualization and Concurrency).

### Rust & Orchestration
Our orchestration, testing, and tooling layers are built in Rust.
*   **Baseline:** Familiarity with `tokio`, `virtmcu-test-runner` library, and Cargo workspace management.

### Toolchain & Version Control
We use `git`, `make`, `meson`, and `ninja`.
*   **Baseline:** You should be able to resolve git conflicts, read a Makefile, and understand the difference between compilation and linking.

## 0.2 Learning Path

If you are coming from:
*   **Theoretical CS/ML:** Start with Fundamentals the SoC Anatomy section and the QEMU Architecture section.
*   **Embedded Engineering:** You can likely skim the MMIO section but must focus on the PDES and Virtual Time section.
*   **Pure Software Engineering:** Pay close attention to the Device Tree section and the QOM section.

## 0.3 Exercises

### Exercise 0.1: Environment Check
Run `make setup` in your devcontainer. Ensure you can build the project and run `make test-check`. If this fails, revisit the Laboratory Setup guide.

### Exercise 0.2: C Pointer Review
Write a small C program that simulates a simple vtable: a struct `Device` containing a function pointer `write`. Instantiate two "objects" with different `write` implementations.

## 0.4 Learning Objectives
After this chapter, you can:
1.  Identify the core technical gaps in your background.
2.  Locate the necessary resources to fill those gaps.
3.  Successfully build and test the VirtMCU workspace.

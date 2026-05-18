# RFC-0022: Fail Loudly vs Linting Policy

## Status
Accepted

## Context

The VirtMCU framework operates under an "Enterprise SOTA" engineering culture with a core mandate: **Fail Loudly**. Because VirtMCU is a deterministic multi-node firmware simulation framework, any internal invariant violation or unexpected state must result in an immediate crash to prevent silent divergence in the simulation. Warnings or graceful degradation for logic errors are considered "code smells."

Simultaneously, the project enforces an extremely strict linting regime (`[workspace.lints.clippy] all = "deny"`), which specifically denies the `clippy::panic` and `clippy::unwrap_used` lints. This strictness is designed to prevent lazy error handling.

This creates a conflict: Developers and AI agents are mandated to crash on invariant violations (via `panic!` or `assert!`), but are blocked by the CI linting pipeline from doing so.

## Decision

We will satisfy both the "Fail Loudly" architectural mandate and the strict linting rules by enforcing a specific hierarchy of error handling and deliberate lint suppression.

1. **For `Option` and `Result` (Developer Errors):**
   * Use `.expect("reason")`.
   * **Rationale:** `clippy::expect_used` is explicitly allowed in the workspace root. It forces the developer to provide a descriptive reason, making the crash informative, unlike `.unwrap()` (which remains banned).

2. **For Logic Invariants and Explicit Panics:**
   * Use `assert!(condition, "reason")` or `panic!("reason")`.
   * To satisfy the linter, the module or file MUST suppress the lint using the mandatory `virtmcu-allow` pattern:
     ```rust
     #![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
     ```
   * **Rationale:** This explicitly signals to reviewers and automated tools that this is a deliberate architectural choice for simulation determinism, rather than a lazy oversight.

3. **For User and Configuration Errors:**
   * Return `Result::Err` or raise specific exceptions.
   * **Rationale:** Expected errors originating from user input must be handled gracefully at the CLI boundary and should provide actionable help before exiting.

4. **Silent Divergence (Warnings):**
   * Emitting warnings for invalid states instead of crashing is strictly **BANNED**. If a state is invalid enough to warn, it is invalid enough to be an error in a deterministic simulation.

## Consequences

* **Clarity:** Developers and AI agents have a clear, documented path to implementing the "Fail Loudly" mandate without fighting the linter.
* **Traceability:** Every explicit panic requires an `expect` string or a `virtmcu-allow` comment, ensuring context is always preserved.
* **Simulation Integrity:** We maintain the strict requirement that simulations fail instantly rather than running with corrupted state, preserving global determinism.

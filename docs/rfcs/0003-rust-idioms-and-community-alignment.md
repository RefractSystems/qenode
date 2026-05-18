# RFC-0003: Rust Idioms and Community Alignment

## Status
Accepted

## Context
VirtMCU is a Rust-based framework built on top of a massive legacy C application (QEMU). While our primary goal is to follow standard Rust community guidelines to ensure maintainability and developer familiarity, the unique constraints of hardware simulation and C-FFI boundaries force us to make intentional architectural compromises.

To ensure transparency for new contributors and community reviewers, this RFC formalizes VirtMCU's relationship with standard Rust idioms.

## Decision: The Philosophy of Rust in VirtMCU

### 1. Adoption of Standard Idioms (Alignment)
VirtMCU explicitly adopts and enforces the following "Standard Rust" patterns:
*   **RAII (Resource Acquisition Is Initialization):** All resources (sessions, timers, memory) must be managed via Rust's ownership and `Drop` semantics. Manual `init`/`deinit` calls are banned.
*   **The Typestate Pattern:** We use the type system to enforce valid program states (e.g., `BqlContext` required for all QEMU state access — a `!Send` zero-sized token that proves BQL ownership at compile time, per RFC-0041).
*   **Dependency Injection (DI):** Component dependencies must be passed explicitly (via `Arc<dyn Trait>`) to ensure testability and parallel simulation safety. Global mutable state is strictly forbidden.
*   **Strict Tooling:** We adhere to the 2021/2024 edition standards and enforce strict `rustfmt` and `clippy` rules.

### 2. Intentional Deviations (The Drift)
Where we drift from standard community guidelines, we do so for deterministic correctness or FFI interoperability. All deviations must be linked to a justifying RFC.

#### A. "Fail Loudly" over "Graceful Results"
*   **Standard Guideline:** "Libraries should never panic; they should return `Result`."
*   **VirtMCU Drift:** We mandate `panic!` or `.expect()` for internal invariant violations that would lead to silent simulation divergence (e.g., corrupted MMIO offsets).
*   **Justification:** [RFC-0022: Fail Loudly vs Linting Policy](0022-fail-loudly-and-panic-linting.md).

#### B. Heavy Procedural Macros for FFI
*   **Standard Guideline:** "Avoid heavy macros that obscure program flow."
*   **VirtMCU Drift:** We use procedural macros (`#[qom_device]`) to generate C-structs and FFI shims.
*   **Justification:** Necessary to bridge QEMU's C Object Model (QOM) safely. Without them, developers would be forced to write thousands of lines of manual, error-prone `unsafe` code. [RFC-0023: Safe QOM Macros](0023-safe-qom-macros.md).

### 3. Encapsulation of `unsafe`
*   **Mandate:** The `unsafe` keyword is permitted only within core framework libraries (like `virtmcu-qom`). 
*   **Requirement:** Peripheral models and high-level logic MUST be 100% safe Rust. Any developer finding a need for `unsafe` in a peripheral should instead propose an enhancement to the framework's safe abstractions.

## Consequences
*   **Positive:** Clear expectations for Rust developers entering the project.
*   **Positive:** Validates our "Enterprise SOTA" claims by grounding our deviations in documented simulation requirements.
*   **Negative:** Developers may initially find the "Fail Loudly" policy jarring compared to standard library development.

## Related
- RFC-0013: Rust as the Primary Language
- RFC-0022: Fail Loudly vs Linting Policy
- RFC-0023: Safe QOM Macros
- RFC-0041: Safe QOM Framework Boundaries via Type-State (`BqlContext` typestate design)

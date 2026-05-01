# Lint Catalogue

VirtMCU employs a strict "fail loudly" philosophy. We use fifteen distinct linters to catch defects before they ever reach a running simulation. This catalogue explains each linter and how to resolve its failures.

## Core Linters

| Name | What it checks | Why | How to fix |
| :--- | :--- | :--- | :--- |
| `check-ffi.py` | Struct layouts and sizes. | Prevents ABI mismatches between Rust and C. | Run `./scripts/check-ffi.py --fix`. |
| `check-qom-alignment.py` | Name equality across 4 layers. | Prevents silent device skip (Bug 2). | Ensure `TypeInfo.name` == `compatible` == Meson `obj` == pytest prefix. |
| `check-cargo-meson-lib-alignment.py` | Cargo name vs Meson `lib`. | Prevents stale `.a` linking. | Align `package.name` in `Cargo.toml` with `lib` in `meson.build`. |
| `clippy` | Rust idiomatic quality. | Prevents common Rust pitfalls. | Follow the suggestions in the `cargo clippy` output. |
| `ruff` | Python code quality. | Ensures consistent Python style. | Run `ruff check --fix .`. |
| `verify-exports.py` | Exported symbols in `.so`. | Ensures QOM can find the plugin. | Check your `module_obj` macro usage. |

## Negative Greps (Banned Patterns)

The build system also greps for patterns that are strictly forbidden in production code.

| Pattern | Reason | Exception Tag |
| :--- | :--- | :--- |
| `thread::sleep` | Breaks virtual time determinism. | `// SLEEP_EXCEPTION` |
| `to_ne_bytes` | Breaks cross-platform determinism. | `// NE_BYTES_EXCEPTION` |
| `print()` | Banned in favor of structured logging. | `// PRINT_EXCEPTION` |
| `#[allow(...)]` | Suppressing warnings is banned. | Requires human approval. |

## Why so many linters?

The "FlexRay Incident" (Postmortem 2026-05-01) showed that when five layers fail silently, the resulting bug is nearly impossible to diagnose. Our linters are designed to turn those silent failures into noisy build errors. If you find a new class of "silent" bug, your first task is to write a linter that catches it.

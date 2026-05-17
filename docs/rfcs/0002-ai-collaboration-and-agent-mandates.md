# RFC-0002: Human-AI Collaboration and Agent Mandates

## Status

**Superseded by `AGENTS.md` (canonical) / `CLAUDE.md` / `GEMINI.md`.**

The agent behavioral mandates described here have been extracted into the workspace instruction files (`AGENTS.md` and its symlinks) that every tool loads automatically. That file is the authoritative, live-updated source; this RFC records the original rationale for the decision to formalize those mandates.

## Summary

This RFC establishes the decision to treat AI-agent behavior in the VirtMCU workspace as an enforceable architectural constraint rather than advisory guidance. The mandates themselves live in `AGENTS.md`; this document records *why* that decision was made and what alternatives were rejected.

## Motivation

VirtMCU is a high-integrity, deterministic simulation framework where a single "lazy" change — using `.unwrap()`, suppressing a lint, or skipping a test — can invalidate a multi-million-dollar firmware verification campaign. As AI agents become primary contributors, their failure modes differ from human developers: hallucination, over-optimization of local context, and pattern-matching shortcuts that look correct but violate deep invariants.

Moving agent rules from "helpful hints in a README" to "machine-readable mandates loaded by the tool on every invocation" converts a process risk into an architectural guarantee. A tool that reads `AGENTS.md` at startup cannot miss the rules; a developer who might skip reading docs might.

## Decision

Agent behavioral rules are encoded in `AGENTS.md` (and its symlinks `CLAUDE.md`, `GEMINI.md`) rather than in a standalone RFC. The key properties of this encoding:

1. **Quality-Only Autonomous Mode**: In `--yolo` mode, agents may only increase quality — suppressing warnings, bypassing types, or using `unsafe` to fix a build is prohibited.
2. **Empirical Verification Gate**: An agent declares work done only after the relevant `make test-check` tier passes. Bug fixes require a failing test before the fix.
3. **Context Fidelity**: Agents read files before editing them; surgical edits over full-file rewrites.
4. **Documentation Responsibility**: Agents flag architectural decisions not covered by an RFC and propose a draft.

## Drawbacks

- Agent instruction files must be kept in sync with the RFC corpus as new design decisions are made. The `AGENTS.md` SSoT mandate mitigates this by making `AGENTS.md` the single file to update.

## Alternatives

- **Enforce mandates via CI lints only**: Catches violations after the fact. Rejected — the goal is to prevent generation of non-conforming code, not just reject PRs.
- **Per-agent configuration files**: Fragmented maintenance. A single `AGENTS.md` loaded by all tools is simpler.

## Unresolved Questions

None. The resolution is `AGENTS.md`.
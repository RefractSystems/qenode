# RFC-0002: Human-AI Collaboration and Agent Mandates

## Summary
This RFC formalizes the engineering standards and behavioral constraints for AI agents (like Gemini CLI) operating within the VirtMCU workspace. It ensures that AI-assisted development preserves "Enterprise SOTA" quality and safety.

## Motivation
VirtMCU is a high-integrity, deterministic simulation framework where a single "lazy" code change (e.g., using `.unwrap()` or skipping a test) can invalidate the results of a multi-million-dollar firmware verification campaign. As AI agents become primary contributors to the codebase, we must move their behavioral rules from "helpful hints" to "architectural mandates."

## The Agent Mandates

### 1. The "Quality-Only" Yolo Rule
When operating in autonomous (`--yolo`) mode, an agent's primary goal is to **increase quality**.
- **No Suppression**: Agents are PROHIBITED from suppressing warnings (e.g., adding `#[allow(...)]`) or bypassing types (e.g., using `as` casts or `unsafe`) simply to fix a build error. They must find the idiomatic, type-safe solution.
- **No Hacks**: Agents must not use reflection, "hidden" logic, or global state to bypass architectural constraints like the DI mandate (RFC-0031).

### 2. Empirical Verification
An agent is not finished with a task until it has empirically proven that the change works.
- **Reproduction**: For bug fixes, the agent MUST first create a failing test case or reproduction script.
- **Verification**: The agent MUST run the full relevant test tier (`make test-check` or `make test-integration`) and verify that all linters pass before declaring victory.

### 3. Absolute Context Fidelity
- **Read-Before-Write**: Agents must never assume the contents of a file or the state of the project. They must use `grep_search` and `read_file` to validate assumptions before applying a `replace`.
- **Atomic Edits**: Agents should prefer surgical `replace` calls over full-file `write_file` calls to preserve unrelated comments, formatting, and surrounding context.

### 4. Documentation Responsibility
If an agent makes an architectural decision not covered by an existing RFC, it must:
1. Flag the decision to the human user.
2. Propose a new RFC draft (as demonstrated by the creation of this document).

## Guide-level explanation
If you are a human developer reviewing an AI's PR:
- Look for the **Validation Gate** in their plan. Did they run `make test-check`?
- Check for `virtmcu-allow` comments. Did the AI justify why it needed to suppress a lint?
- If the AI used `.unwrap()`, reject the PR immediately. It violated the "Fail Loudly" mandate (RFC-0022).

## Reference-level explanation
These mandates are enforced via:
- **`GEMINI.md`**: The source of truth for agent instructions.
- **Custom Linters**: The `virtmcu-test-runner` includes lints that catch "lazy AI" patterns (e.g., banning `thread::sleep` and `unwrap`).
- **CI Gates**: The PR pipeline will reject any change that violates these mandates, regardless of whether it was authored by a human or an AI.

## Rationale and alternatives
We could treat AI agents as standard developers. However, agents have different failure modes (hallucination, over-optimization of context, laziness). Specific mandates counteract these tendencies and ensure the AI acts as a "Senior Staff Engineer" rather than a "Junior Intern."

## Unresolved questions
- How do we handle agents with smaller context windows that cannot read the entire RFC library? (Proposed: Use `update_topic` to maintain a "summarized architectural state" in the session history).
- Should we implement automated "Agent Grading" in CI?
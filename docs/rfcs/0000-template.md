# RFC-XXXX: [RFC Title]

> **Numbering:** Do not assign a number to your draft. VirtMCU uses the **GitHub PR number** as the RFC ID.
> 1. Name your file `XXXX-my-topic.md` and open a Pull Request.
> 2. Once GitHub assigns a PR number (e.g., `#142`), rename to `0142-my-topic.md` and update the title.

## Status

Draft

## Summary

One paragraph explaining the decision: what is being decided and why it matters.

## Motivation

Why is this decision needed? What problem does it solve? What bugs, limitations, or constraints drove the design? Concrete failure scenarios are more convincing than abstract principles.

## Detailed Design

The technical substance of the RFC. Address:

- The proposed design, with enough detail to assess correctness.
- How it interacts with the BQL, simulation determinism, and other framework invariants.
- Edge cases and how they are handled.

Code examples belong here, not in Motivation.

## Drawbacks

Why should we *not* do this? Every design has costs — incomplete drawbacks are a red flag that the proposal is not well-reasoned.

## Alternatives

What other designs were considered? Why were they rejected? What is the cost of doing nothing?

## Prior Art

Relevant prior work from QEMU, Renode, gem5, or Rust ecosystem. Good and bad precedents both belong here.

## Unresolved Questions

- What parts of the design require resolution through the RFC process before acceptance?
- What parts require resolution during implementation?
- What is explicitly out of scope?

If there are no unresolved questions, write "None." Empty sections look incomplete.

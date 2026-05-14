# RFC-XXXX: [RFC Title]

> **Note to Authors on Numbering:** 
> Do not assign a sequential number to your RFC draft. VirtMCU uses the **GitHub Pull Request number** to assign RFC IDs. 
> 1. Copy this template and name your file `XXXX-my-feature.md`.
> 2. Open a Pull Request with your draft.
> 3. Once GitHub assigns a PR number (e.g., `#142`), rename your file to `0142-my-feature.md` and update the title to `RFC-0142: [RFC Title]`.

## Summary
One paragraph explanation of the feature.

## Motivation
Why are we doing this? What use cases does it support? What is the expected outcome?

## Guide-level explanation
Explain the proposal as if it was already included in the framework and you were teaching it to another programmer. That generally means:

- Introducing new named concepts.
- Explaining the feature largely in terms of examples.
- Explaining how developers should *think* about the feature.

If applicable, describe the differences between teaching this to a VirtMCU core developer vs. an end-user integrating with Firmware Studio.

## Reference-level explanation
This is the technical portion of the RFC. Explain the design in sufficient detail that:

- Its interaction with other features is clear.
- It is reasonably clear how the feature would be implemented.
- Edge cases are called out, including how this interacts with the Big QEMU Lock (BQL) and simulation determinism.

## Drawbacks
Why should we *not* do this?

## Rationale and alternatives
- Why is this design the best in the space of possible designs?
- What other designs have been considered and what is the rationale for not choosing them?
- What is the impact of not doing this?

## Prior art
Discuss prior art, both the good and the bad, in relation to this proposal. Are there lessons we can learn from how QEMU, Renode, or other simulation frameworks solved this problem?

## Unresolved questions
- What parts of the design do you expect to resolve through the RFC process before this gets merged?
- What parts of the design do you expect to resolve through the implementation of this feature before stabilization?
- What related issues do you consider out of scope for this RFC that could be addressed in the future independently of the solution that comes out of this RFC?
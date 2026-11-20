# Phase 1 research plan

Complete and review each exit gate before advancing. Phases 1.0 through 1.3
are accepted. Phase 1.4 met its exit gate and is ready for maintainer review.
Phase 1.5 awaits maintainer authorization. Record source-derived expectations
and observed results with their exact inputs, dependency versions, and commands.
Use [Phase 0](../README.md) and
[ADR-001](../../../docs/adr/001-evm-guest-state-abi.md) as comparison references.

## 1.0 — Experimental scaffolding

- **Research question.** Can the spike build as a non-default workspace member using the existing assembler pin?
- **Expected evidence.** A frozen build, the resolved feature tree, a lockfile comparison against the baseline, and the baseline workspace test inventory.
- **Exit gate.** The scaffold builds, only the spike package entry is added to the lockfile, and the workspace reports the same four named regression failures alongside 137 passes.

## 1.1 — Assembler result API

- **Status.** Accepted. The specimen reproduced the expected bytes and exposed absolute label addresses and an output-relative applied relocation. The production baseline reproduced all four known failures, and the spike test passed. [Results](results/1.1-assembler-api.md)
- **Research question.** What final bytes, label addresses, and applied relocation details does the pinned assembler expose for RV64 at a nonzero origin?
- **Expected evidence.** A minimal labeled input with recorded source, origin, result bytes, reported addresses and relocation entries, and the corresponding pinned API source.
- **Exit gate.** Reproduce the result and establish the address and offset conventions, or record the missing information and stop for review.

## 1.2 — Symbolic control flow

- **Status.** Exit gate met. Six experiments verified symbolic JAL displacements, encoding boundaries, alignment and named-symbol diagnostics, and conditional-branch relaxation. The production baseline reproduced all four known failures. [Results](results/1.2-symbolic-control-flow.md)
- **Research question.** Which references and input constraints does the assembler handle, and how does relaxation affect final layout?
- **Expected evidence.** Forward and backward symbolic JALs, signed range edges and just-outside cases, misalignment, duplicate and undefined labels, and short and relaxed conditional branches. Record final bytes, label positions, applied relocations, and raw diagnostics.
- **Exit gate.** Establish the supported reference forms and layout behavior, including the distinction between ISA alignment and Ferreth's four-byte instruction alignment. Record any blocking assembler limitation for review.

## 1.3 — Independent instruction verification

- **Status.** Accepted. Six experiments verified byte-derived JAL and branch targets, including the two-word relaxed BEQ and rejection of the LF-027 sign-bit corruption. The production baseline reproduced all four known LF-026 failures. [Results and RF-011](results/1.3-independent-verification.md)
- **Research question.** Can final RV64 control transfers be checked against their intended destinations independently of the assembler's relocation calculations?
- **Expected evidence.** Hand-derived instruction fixtures and checks for opcode, destination register where applicable, signed displacement, site alignment and bounds, and target PC from the instruction's own address. Include an intentionally wrong destination and a backward JAL that exercises LF-027.
- **Exit gate.** The checks accept the valid forward and backward fixtures and reject the wrong-target and malformed cases. The comparison of final words to intended addresses is reproducible.

## 1.4 — Symbolic PUSH1 composition

- **Status.** Exit gate met. One, two, and five PUSH1 fragments assembled with unique local labels and independently verified transfers. The two-PUSH artifact matched production bytes and publisher PCs. Short normal and full-stack exception runs matched production results, and the workspace reproduced the four known LF-026 failures. [Results and RF-012](results/1.4-symbolic-push1.md)
- **Research question.** Can repeated PUSH1 bodies bind unique local labels and symbolic normal and overflow destinations while preserving short-guest behavior?
- **Expected evidence.** Final source, words, publisher PCs, and independently checked control transfers for short specimens. Compare EOF and STOP cases and short full-stack overflow with Phase 0 for status, stack, final `s1`, and RV64 instruction counts.
- **Exit gate.** Every checked reference binds to its intended target, the short specimens satisfy the ABI and match baseline semantics, and differences in assembly, words or counts have an explanation.

## 1.5 — Long-range guest execution

- **Research question.** Does symbolic assembly resolve the LF-026 boundary and execute an actually taken distant overflow transfer?
- **Expected evidence.** Run `6002` repeated 46 and 47 times, at EOF and followed by STOP, from an empty stack. Run both 47-PUSH variants from the Phase 0 seeded 1,024-word stack and verify the first overflow destination from the final word and the observed publisher PC.
- **Exit gate.** All four empty-stack cases complete with the expected number of twos. Both seeded runs publish overflow, preserve all 1,024 seeded words and `s1 = 0x80109000`, and reach the independently verified overflow publisher.

## 1.6 — Mixed opcode composition

- **Research question.** Can repeated ADD, SUB, POP and DUP1 fragments compose with PUSH1 while preserving local arithmetic calls, continuation targets, and exception routing?
- **Expected evidence.** Short representative mixed guests with repeated fragments, EOF and STOP, seeded inputs and exceptional cases, including both DUP1 exits. Record final references, outcomes, and instruction counts against Phase 0 controls where available. Compare standalone ADD/SUB core hashes with Phase 0.
- **Exit gate.** Reference checks establish that each occurrence calls its own leaf and reaches its own continuation where applicable, both DUP1 exits reach their intended publishers, and mixed guest outcomes satisfy the ABI. The standalone arithmetic core hashes match.

## 1.7 — Architecture evaluation

- **Research question.** Does the measured result justify symbolic assembly alone, or does independent validation require a minimal explicit record of intended references?
- **Expected evidence.** A comparison grounded in the preceding probes, with supported references, limits, required metadata, ABI and compatibility implications, and traceable regression requirements for any follow-on implementation.
- **Exit gate.** Record a reviewable recommendation supported by the evidence, or document the blocking capability and the smallest checked patching alternative. Production integration requires a separate maintainer decision.

# Verification Policy

| Field | Value |
|---|---|
| Document type | Common verification policy |
| Version | 0.1 |
| Status | Proposed |
| Scope | RTL and hardware-interface milestones |

An approved mission states which of these checks apply to its deliverables and their acceptance thresholds. Verification results must identify the artifact revision and evidence; agent self-report alone is not evidence.

## Required practices

1. **Lint and compile:** lint and compile RTL with the mission's declared toolchain. Record tool versions, compiler arguments, relevant SystemVerilog language assumptions, warnings, and commands. The mission must state warning policy.
2. **Directed and boundary tests:** exercise ordinary values and relevant extremes, boundary transitions, operation decoding, and invalid inputs as defined by the interface.
3. **Deterministic randomized tests:** record seeds and test configuration. Reproduce failures using the same input and seed; do not rotate a seed because it found a failure. Additional predefined seeds may add coverage without replacing failing cases.
4. **Independent reference models:** compare against software models controlled independently from agent-generated RTL. Keep evaluator-only tests and protected acceptance criteria outside team modification scope.
5. **Regression:** rerun the prescribed regression after relevant changes and retain the result, including failures.
6. **Simulator evidence:** record simulator and compiler versions, invocation and arguments, RTL revision, test commands, random seeds, and execution results. Preserve failure inputs and useful logs or waveforms.
7. **Reproducibility:** retain enough configuration and provenance for another operator to repeat the run. Report unavailable evidence and deviations rather than implying they were measured.

## Evaluation-asset independence

Reference models, required acceptance criteria, and evaluator-only tests must be controlled independently of the agent teams. Teams may not alter protected criteria or hidden evaluator tests. The evaluator should use deterministic tests and record the exact test-suite revision and execution configuration. Any test exposure or contamination must be reported.

## Interpreting results

Passing simulation demonstrates only that the simulated design matched the tested expectations for the recorded conditions. It is not a formal proof, an ISA or Ethereum conformance claim, a synthesis result, or evidence of a physical implementation.

Synthesis and formal verification are future options, not assumed ETH-001 obligations. If introduced, record tool versions, constraints, assumptions, properties, proof boundaries, and results. Formal proof claims must name what was proved and under which model and assumptions; simulation and test coverage must not be described as proof. Synthesis estimates are tool- and constraint-dependent and do not establish measured silicon area, timing, power, or physical implementation.

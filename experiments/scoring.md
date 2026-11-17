# Tournament Scoring

| Field | Value |
|---|---|
| Document type | Experiment scorecard policy |
| Version | 0.2 |
| Status | Proposed |
| Scope | Optional tournament scorecards and separate reporting of research outcomes |

This policy supports optional agent-workflow experiments; it does not make tournaments the primary Ferrum ETH objective. Score hardware quality and agent engineering performance as separate scorecards. Do not combine them into a single team ranking. Publish the evidence, rubric, and any weights before trials begin; this document assigns no scores to teams and reports no results.

## 1. Research outcome reporting

Research findings are not tournament scores. Report EVM-to-RISC-V results by named specification/fork, executable reference and revision, exact bytecode, tested observables, test counts, mismatches, and limitations. Distinguish empirical parity on tested cases from specification conformance and formal correctness.

For hardware comparisons, report correctness separately from the measured metrics and state the workload, baseline, accounting boundary, tools, constraints, and unavailable measurements. Suitable metrics may include instruction count, simulated cycles, synthesized area and timing, and eventually energy. Include interface/data-movement costs when measured. Do not infer that a specialized implementation is faster or more efficient, and do not combine unlike measurements into an unqualified score.

## 2. Hardware Engineering Quality

### Objective gates

A submission is eligible to win a hardware category only if it passes every required correctness and acceptance check in the frozen mission, including applicable directed, boundary, deterministic randomized, regression, and independent evaluator tests. Required lint/compile or reproducibility gates also apply when specified by the mission. A correct hardware implementation is a prerequisite to any winning result; resource use or qualitative merit cannot compensate for a required correctness failure.

Classify a submission as **incomplete** when required deliverables or evidence are missing, and **invalid** when it violates frozen scope, changes protected criteria/evaluator assets, or cannot be evaluated under the predeclared procedure. Incomplete and invalid submissions do not win a hardware category. Report observed partial results and the reason for the classification rather than inventing a score.

### Quantitative measures

Report, with raw counts and test/configuration revisions:

- Required tests passed, failed, and not run; evaluator-only failures; and regression status.
- Lint and compile outcomes and warning counts under the declared policy.
- Defects found during review and their predeclared severity classification.
- For future synthesis milestones only: area, timing, and power measurements or estimates under fixed tools, target, constraints, and methodology. Do not report unperformed measurements or call estimates physical results.

### Qualitative review

Use a predeclared rubric, preferably blinded, to assess contract clarity, maintainability, traceability to requirements, implementation simplicity, and quality of failure/reproducibility evidence. Explain ratings with specific evidence. Qualitative ratings cannot waive objective gates.

## 3. Agent Engineering Performance

Measure workflow performance separately from the hardware artifact. Report quantitative measures such as elapsed time, inference/token or other available resource use, tool invocations, retries, completion rate, handoff/rework count, and reproducibility of the run. State the accounting method and unavailable measurements; do not treat incomparable counters as equivalent.

Qualitative review may assess process discipline, useful review findings, handoff clarity, adherence to authorized scope, and provenance completeness. Use the same published rubric and evidence limits across teams.

An agent-performance result does not establish hardware quality. Teams whose hardware fails required correctness gates may still have workflow measurements reported, but cannot be declared hardware winners. Do not combine the scorecards into a single result that lets agent efficiency outweigh incorrect hardware.

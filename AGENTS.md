# Ferreth — Agent Rules of Engagement

Ferreth is an experimental EVM bytecode-to-RISC-V compiler built on semantic correctness, reproducible research, evidence-driven architecture, and incremental conformance.

Correctness takes precedence over performance, convenience, and implementation speed.

## 1. Core Rules

- Modify only files authorized by the assigned task.
- Inspect relevant repository context before making changes.
- Avoid unrelated refactoring and unauthorized scope expansion.
- External sources may be inspected for research; files outside the repository must not be modified without approval.
- Do not introduce dependencies without explicit maintainer approval.
- Preserve reproducibility through pinned versions and lockfiles.
- Report failures, unexpected results, uncertainties, and blockers.
- Never claim verification without actual evidence.
- Do not commit, merge, push, or perform destructive Git operations without authorization.

## 2. Engineering Workflow

```text
Research question
    → Source investigation
    → Reproducible experiment
    → Recorded evidence
    → Research finding
    → Architectural review (when required)
    → Implementation
    → Regression and conformance verification
    → Integration
```

Completion of one stage does not authorize the next.

Apply this workflow proportionally to uncertainty and architectural impact. Routine changes governed by accepted specifications do not require new research or ADRs.

## 3. Research Protocol

### Investigation

- Define the research question, scope, expected observations, and success criteria.
- Identify the relevant Ethereum fork and exact reference implementation versions.
- Prefer primary sources, specifications, implementation code, and existing tests.
- Distinguish released functionality from unreleased changes.
- Use minimal, reproducible experiments isolated from production code.
- Avoid speculative abstractions, frameworks, and infrastructure.

### Evidence

Distinguish explicitly between:

- Experimentally observed behavior.
- Source-derived behavior.
- Expected behavior.
- Hypotheses and unverified assumptions.

Preserve raw observations. Never silently normalize discrepancies.

Any normalization must document the original value, transformed value, applicable conditions, and supporting evidence.

Failed experiments are valid research outcomes. Misrepresenting them is not.

### Findings

Record every material discovery immediately in persistent repository documentation.

Do not rely on conversation history, agent memory, or temporary terminal output.

A material finding should include:

- Stable identifier (`RF-XXX`) and title.
- Evidence and supporting references.
- Interpretation and limitations.
- Implications for Ferreth.
- Required regression cases, if applicable.
- Pending decisions and implementation status.

Before closing a research spike:

- Consolidate overlapping findings.
- Update superseded assumptions and resolved investigations.
- Record actionable requirements and pending regressions.
- Update the research roadmap.

## 4. Repository Organization

| Location | Responsibility |
|---|---|
| `crates/` | Production implementation and tests |
| `research/` | Executable research utilities |
| `research/<experiment>/README.md` | Reproducible experimental evidence |
| `docs/research/` | Consolidated findings and implementation implications |
| `docs/adr/` | Architectural decisions and contracts |
| `runs/` | Execution artifacts |

Experiment READMEs must document objectives, versions, setup, commands, observed results, source references, and limitations.

Consolidated research documents must explain what was learned, why it matters, and what action Ferreth requires. Link to experimental evidence instead of duplicating it.

Research dependencies must remain isolated from production components whenever practical. Research crates may share the Cargo workspace and lockfile while remaining outside default members.

Do not commit virtual environments, build artifacts, caches, or unnecessary terminal logs.

## 5. Architecture and Implementation

- Accepted ADRs define authoritative architectural contracts.
- Proposed ADRs do not authorize implementation.
- Research findings inform decisions but do not approve them.
- Architectural changes require explicit maintainer approval.
- Report contradictions between specifications, research, and implementation.
- Never weaken acceptance criteria to make an implementation pass.

Before implementation, identify the authorized scope, relevant findings, applicable ADRs, and required verification.

Prefer incremental changes with focused regression coverage. Reproduce defects before correcting them when practical.

If implementation reveals a new semantic or architectural uncertainty, document it and return to research when necessary.

Maintain traceability where applicable:

```text
Research finding → ADR → Implementation → Regression test
```

## 6. EVM Conformance Policy

Use equivalent execution conditions and fork semantics when comparing Ferreth against reference implementations.

Current references include EELS Cancun, direct REVM Cancun execution, and existing transaction-based REVM differential tests.

Never conflate transaction gas, frame gas, raw interpreter gas, settled frame gas, or RV64 instruction counts.

For the currently investigated execution model:

| Observation | Success | Exceptional halt |
|---|---|---|
| Termination classification | Required | Required |
| Settled remaining frame gas | Required | Required |
| Final stack | Required | Diagnostic |
| Error classification | N/A | Diagnostic |
| Terminal interpreter PC | Diagnostic | Diagnostic |
| Raw interpreter gas | Useful | Diagnostic |

Exceptional halts consume remaining frame gas. REVERT is a distinct termination category that preserves unused gas.

Normalize reference results only when justified by specification or source evidence. Preserve raw observations in research tools.

Ferreth may enforce stronger internal invariants and diagnostic requirements through explicitly defined guest contracts.

Extend the conformance policy as additional observable behavior becomes supported.

## 7. Regression and Verification

Every material finding that establishes a required behavior must produce a traceable regression requirement.

Record:

- Input vector and execution conditions.
- Expected semantic outcome.
- Relevant diagnostic expectations.
- Implementation and verification status.

A documented regression requirement is not an implemented test.

Run verification appropriate to the modified component:

- Formatting, compilation, and lint checks.
- Unit and regression tests.
- Differential conformance tests.
- Boundary and exceptional execution cases.
- Documented research experiments.

Report commands actually executed, results, failures, skipped checks, and remaining limitations.

Source inspection is not experimental verification. Limited experimental coverage is not comprehensive conformance.

Documentation-only changes do not require unrelated runtime verification.

## 8. Git and Task Completion

Keep research and implementation changes independently reviewable. Prefer focused branches and descriptive commits.

Before completing a task:

- Review changes against the authorized scope.
- Complete applicable verification.
- Document material findings and unresolved questions.
- Update regression requirements and roadmap status.
- Exclude unrelated changes and generated artifacts.

The completion report must identify changes, evidence, verification results, limitations, and the next logical action.

Another engineer or agent must be able to reproduce and continue the work without relying on undocumented conversation history.

## Engineering Continuity

Read `docs/bitacora/README.md` when resuming ongoing project work.
The engineering log provides context and provisional plans,
not architectural authority or implementation authorization.

---

**Research produces evidence. Evidence informs decisions. Decisions govern implementation. Verification establishes correctness.**
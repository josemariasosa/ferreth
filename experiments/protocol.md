# Tournament Protocol

| Field | Value |
|---|---|
| Document type | Reproducible experiment protocol |
| Version | 0.1 |
| Status | Proposed |
| Scope | Comparison of agent-team workflows on approved missions |

This protocol defines controls for comparing Team A (Solo Engineer), Team B (Sequential Specialists), and Team C (Adversarial Review). It does not implement or authorize an orchestrator. Mission-specific requirements and acceptance criteria come from the approved, frozen mission.

## Before a trial

The human experiment owner approves and records:

- Experiment ID, mission ID/version, frozen mission and acceptance-test revisions, and identical starting Git commit for all teams.
- One isolated Git worktree per team and trial, created from that same commit.
- The exact allowed files, tools, evaluator inputs, and test visibility.
- The local model identity and available version or digest, quantization, runtime, configuration, and relevant host configuration.
- A comparable total resource budget across team architectures, with accounting units, time limits, and tool/inference limits specified before execution.
- A maximum retry count and the conditions under which a retry is permitted. Retries do not replace or erase a failed attempt.
- Session, prompt, and memory boundaries, including controls against cross-team context, shared mutable files, and premature result sharing.
- Deterministic independent evaluation inputs and the method for preserving protected acceptance tests.

Any conditions that cannot be made identical must be declared in advance with the rationale and comparison limitation. Do not change the mission or acceptance criteria during a trial.

## Execution and provenance

- Give every team the same mission context, source revision, evaluator-independent public material, and total resource budget. Team-specific role structure may differ as defined for the experiment.
- Keep worktrees, sessions, mutable memory, logs, and outputs isolated between teams and trials until all submissions are frozen.
- Record model and runtime configuration, role prompts, session identifiers, tool versions/configuration, resource use, elapsed time, retries, input and output revisions, and handoff artifacts as available.
- Freeze each submission by recording its source revision and artifact hashes before independent evaluation.
- Evaluate all submissions using the same deterministic procedure and protected test-suite revision. Evaluators should be blinded to team identity where feasible.
- Record deviations, infrastructure failures, missing measurements, and contamination or isolation concerns; do not silently discard unfavorable runs.

**A Git worktree is version-control separation, not operating-system security isolation.** It does not by itself prevent processes from accessing other paths, shared model state, network services, caches, or credentials. Document actual controls and residual risks; do not claim stronger isolation than was implemented.

## Repeated trials and archival

Run repeated independent trials when resources permit. A single trial is descriptive and does not establish a general team capability. Report trial counts, variation, outliers, and deviations without overstating statistical conclusions.

Archive the preregistration, exact starting and submission revisions, mission and evaluator revisions, model/tool configuration, resource records, test results, logs, and analysis through an approved repository strategy. Preserve raw results alongside any summaries. Record what could not be captured and known contamination risks, including shared caches or pretraining, nondeterministic inference, evaluator leakage, model changes, reused context, and unequal effective retries.

Use [the scoring policy](scoring.md) to report hardware quality and agent performance separately.

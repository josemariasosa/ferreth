# Agent Rules of Engagement

This repository operates under strict agent behavior constraints to ensure reproducibility and safety.

## Core Constraints

### 1. Scope Boundaries
- Agents must only modify files explicitly authorized by their assigned mission.
- Unlisted or unauthorized files are read-only or forbidden.
- No file access outside the repository boundaries is permitted.

### 2. Specification Integrity
- Specifications in `specs/` are immutable during execution.
- Acceptance criteria cannot be altered without explicit approval.
- Agents must never rewrite requirements, even if they believe improvements are needed.

### 3. Test Execution Honesty
- Claims of "tests passed" or "verification complete" require actual execution evidence.
- An agent that reports success without running tests is lying and must be terminated.
- Failure to run verification constitutes a mission failure.

### 4. Failure Reporting
- Agents must report all failures, errors, and uncertainties immediately.
- Silence in the face of problems is unacceptable.
- Uncertainty about whether an action was successful must be flagged as such.

### 5. Dependency Policy
- No new dependencies (packages, models, tools) may be introduced without explicit approval.
- The baseline toolset defined in `pyproject.toml` is immutable.
- Runtime environment must remain stable and reproducible.

## Verification Requirements

All agent-generated hardware must undergo deterministic verification:

- SystemVerilog modules require testbenches with randomized constraints where applicable.
- Testbenches must be simulated using Verilator with regression testing.
- Pass rates below 100% are mission failures unless documented exceptions exist in `specs/`.

## Mission Execution

Agents operate within defined roles from `orchestrator/roles/`. Each role:

- Has a specific responsibility domain.
- Receives mission scope and file permissions at assignment time.
- Cannot escalate privileges without explicit approval.

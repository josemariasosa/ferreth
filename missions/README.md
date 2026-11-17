# Missions

An executable mission is an approved, versioned work package with bounded authority and testable acceptance criteria. It turns a project objective into a specific assignment; it is not an architecture document and must not silently revise general architecture, repository safety rules, or experiment-integrity constraints.

## Required mission contents

Every mission specification must include:

- Mission ID and version, status, and owner/approval record.
- Objective and explicit scope, including authorized file paths.
- Inputs, starting revision, and context/reference documents with their revisions where available.
- Hardware interface requirements and functional semantics, including widths and relevant edge cases.
- Deliverables and verification requirements, including commands/evidence to retain.
- Objective, testable acceptance criteria and protected evaluator assets.
- Explicit non-goals.
- Resource, time, and retry limits.
- Evidence and provenance requirements.

Role assignments and stage/handoff responsibilities may be included when relevant, but do not grant authority beyond the mission's authorized paths or the repository-wide rules.

## Lifecycle and precedence

The human owner approves and freezes the mission before execution. Trials identify that exact mission revision and starting commit. Changes to requirements or acceptance criteria require explicit approval and a new version; never edit a frozen mission to accommodate a result.

An approved mission governs mission-specific details over general architecture guidance. It cannot override repository-wide safety, protected-test, or experiment-integrity constraints. If an unresolved conflict remains, pause for human resolution.

## Suggested structure

```markdown
# <Mission ID> — <title>

| Field | Value |
|---|---|
| Version | <version> |
| Status | Draft / Approved / Frozen |
| Owner | <human owner> |

## Objective
## Scope and authorized paths
## Inputs and context references
## Hardware interface requirements
## Functional semantics
## Deliverables
## Verification requirements
## Acceptance criteria
## Explicit non-goals
## Resource and retry limits
## Evidence and provenance
```

Context documents may explain background for offline agents, but are not executable specifications and do not add acceptance criteria.

# ferrum-eth

Exploratory research into EVM-to-RISC-V behavioral parity and Ethereum-oriented hardware acceleration. Agent orchestration is a supporting experimental methodology, not the primary scientific objective.

## Research Tracks

1. **EVM-to-RISC-V translation and behavioral parity** - Begin with small EVM bytecode sequences translated to RV64IM and compare observable behavior with a pinned executable reference. Yul is a possible later path, with its compiled EVM bytecode explicitly identified.
2. **Ethereum-oriented hardware acceleration** - Compare conventional RV64IM implementations with specialized RTL or accelerators under defined workloads; no performance or efficiency advantage is assumed.
3. **Agent-workflow experiments** - Optionally compare orchestration methods on approved missions without treating tournament outcomes as evidence of parity or hardware advantage.

## Workflow

- Specifications in `specs/` define mission requirements and acceptance criteria.
- Agents operate from `orchestrator/` with defined roles and permissions.
- Experiments in `experiments/` record research methods and, when separately approved, agent-team comparisons.
- Hardware RTL goes to `hardware/rtl/`, simulation artifacts to `hardware/sim/`.
- Completed runs land in `runs/` (ignored by Git).

## Repository Rules

See `AGENTS.md` for agent constraints and operational boundaries.

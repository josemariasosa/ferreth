# ferrum-eth

Experimental laboratory combining AI agent orchestration with RISC-V and Ethereum-oriented hardware design.

## Research Tracks

1. **AI Agent Orchestration** - Using local LLMs to coordinate task execution across specialized agents.
2. **Hardware Design & Verification** - SystemVerilog RTL development for RISC-V and Ethereum-related components, verified via Verilator.

## Workflow

- Specifications in `specs/` define mission requirements and acceptance criteria.
- Agents operate from `orchestrator/` with defined roles and permissions.
- Experiments in `experiments/` compare different agent team architectures.
- Hardware RTL goes to `hardware/rtl/`, simulation artifacts to `hardware/sim/`.
- Completed runs land in `runs/` (ignored by Git).

## Repository Rules

See `AGENTS.md` for agent constraints and operational boundaries.

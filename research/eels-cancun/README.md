# EELS Cancun — Frame Execution Research

## Objective

Establish a reproducible, independent execution reference for evaluating Ferreth's EVM semantics against the Ethereum Execution Layer Specification (EELS).

This experiment executes EVM bytecode directly through the Cancun `process_message()` entry point, bypassing transaction processing and intrinsic gas accounting.

The primary observations are:

- Execution success or exceptional halt.
- Remaining and consumed frame gas.
- Final stack.
- Program counter.
- Error classification.

This research supports Ferreth's incremental EVM conformance roadmap and the future implementation of ADR-002.

## Environment

- Ethereum fork: Cancun
- Python: 3.14
- Package manager: uv
- Package: `ethereum-execution`
- Repository: https://github.com/ethereum/execution-specs
- Pinned revision: `d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3`
- Experiment date: 2026-11-18

The Python environment and dependencies are managed through `.python-version`, `pyproject.toml`, and `uv.lock`.

## Setup

From this directory:

```bash
uv sync --locked
```

## Usage

```bash
uv run run_frame.py --code 600260030100 --gas 20
```

Arguments:

| Argument | Description |
|---|---|
| `--code` | Hexadecimal EVM bytecode, with optional `0x` prefix. |
| `--gas` | Initial execution frame gas. |
| `--tx-gas` | Optional transaction environment gas. Defaults to frame gas. |

Each invocation constructs a fresh in-memory Ethereum state and synthetic block, transaction, and message environments.

The runner does not connect to an Ethereum node or require historical blockchain data.

### Example output

```text
Bytecode: 600260030100
Initial txEnv gas: 20
Initial Msg gas: 20
Remaining gas: 11
Gas consumed: 9
Stack: [5]
PC: 6
Success: True
Error: None
Error type: None
Pre-state root: 56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421
```

## Experimental Results

All results in this section were observed through actual execution against EELS Cancun on 2026-11-18.

Unless explicitly specified, transaction environment gas equals initial frame gas.

Stack values are represented from bottom to top.

### 1. Successful execution

```bash
uv run run_frame.py --code 00 --gas 0
uv run run_frame.py --code 600200 --gas 20
uv run run_frame.py --code 600260030100 --gas 20
uv run run_frame.py --code 600260030300 --gas 20
uv run run_frame.py --code 60025000 --gas 10
uv run run_frame.py --code 60028000 --gas 10
```

| Program | Bytecode | Initial gas | Remaining gas | Consumed | Stack | PC |
|---|---|---:|---:|---:|---|---:|
| STOP | `00` | 0 | 0 | 0 | `[]` | 1 |
| PUSH1 | `600200` | 20 | 17 | 3 | `[2]` | 3 |
| ADD | `600260030100` | 20 | 11 | 9 | `[5]` | 6 |
| SUB | `600260030300` | 20 | 11 | 9 | `[1]` | 6 |
| POP | `60025000` | 10 | 5 | 5 | `[]` | 4 |
| DUP1 | `60028000` | 10 | 4 | 6 | `[2, 2]` | 4 |

All executions completed successfully without errors.

### 2. Frame gas independence

```bash
uv run run_frame.py --code 600200 --gas 3 --tx-gas 3
uv run run_frame.py --code 600200 --gas 3 --tx-gas 0
uv run run_frame.py --code 600200 --gas 2 --tx-gas 100000
```

| TX environment gas | Frame gas | Remaining gas | Stack | PC | Error |
|---:|---:|---:|---|---:|---|
| 3 | 3 | 0 | `[2]` | 3 | None |
| 0 | 3 | 0 | `[2]` | 3 | None |
| 100000 | 2 | 0 | `[]` | 0 | OutOfGasError |

These observations demonstrate that the initial execution budget for this direct interpreter path is determined by message gas, not transaction environment gas.

Synthetic mismatches between transaction and frame gas are intentional experimental controls, not valid transaction-processing contexts.

### 3. Exceptional halts and error precedence

```bash
uv run run_frame.py --code 01 --gas 0
uv run run_frame.py --code 01 --gas 2
uv run run_frame.py --code 01 --gas 3
uv run run_frame.py --code 80 --gas 3
uv run run_frame.py --code 600260030100 --gas 8
uv run run_frame.py --code 60020100 --gas 20 --tx-gas 100000
```

| Bytecode | Initial gas | Remaining gas | Consumed | Stack | PC | Error |
|---|---:|---:|---:|---|---:|---|
| `01` | 0 | 0 | 0 | `[]` | 0 | StackUnderflowError |
| `01` | 2 | 0 | 2 | `[]` | 0 | StackUnderflowError |
| `01` | 3 | 0 | 3 | `[]` | 0 | StackUnderflowError |
| `80` | 3 | 0 | 3 | `[]` | 0 | StackUnderflowError |
| `600260030100` | 8 | 0 | 8 | `[]` | 4 | OutOfGasError |
| `60020100` | 20 | 0 | 20 | `[]` | 2 | StackUnderflowError |

All executions terminated exceptionally.

Key observations:

- ADD checks stack operands before charging its ordinary opcode gas in EELS.
- Empty-stack DUP1 produces `StackUnderflowError`.
- Exceptional halts consume all remaining frame gas.
- Failed instructions may leave partially modified internal stacks.
- Error precedence and exceptional stack contents should be treated as implementation diagnostics when comparing independent interpreters.

### 4. Truncated PUSH1

```bash
uv run run_frame.py --code 60 --gas 3
```

Observed result:

```text
Bytecode: 60
Initial txEnv gas: 3
Initial Msg gas: 3
Remaining gas: 0
Gas consumed: 3
Stack: [0]
PC: 2
Success: True
Error: None
Error type: None
```

A missing PUSH1 immediate is interpreted as zero. Execution completes successfully, even though the final program counter exceeds the bytecode length.

Ferreth's existing truncated-PUSH1 rejection must be addressed as a separate conformance correction.

### 5. GAS opcode

```bash
uv run run_frame.py --code 5a00 --gas 10
uv run run_frame.py --code 5a5a00 --gas 10
uv run run_frame.py --code 5a00 --gas 2
uv run run_frame.py --code 5a00 --gas 1
uv run run_frame.py --code 5a00 --gas 0
```

| Bytecode | Initial gas | Remaining gas | Consumed | Stack | PC | Error |
|---|---:|---:|---:|---|---:|---|
| `5a00` | 10 | 8 | 2 | `[8]` | 2 | None |
| `5a5a00` | 10 | 6 | 4 | `[8, 6]` | 3 | None |
| `5a00` | 2 | 0 | 2 | `[0]` | 2 | None |
| `5a00` | 1 | 0 | 1 | `[]` | 0 | OutOfGasError |
| `5a00` | 0 | 0 | 0 | `[]` | 0 | OutOfGasError |

The GAS opcode charges its own cost of two gas units before pushing the remaining frame gas onto the stack.

Repeated GAS instructions observe the decreasing execution budget.

## Conformance Implications

The following behaviors should be covered by Ferreth's incremental conformance tests:

- Exact gas consumption for successful execution.
- Zero-cost STOP execution.
- Successful execution at exact gas boundaries.
- Exceptional halt with insufficient gas.
- Exceptional halt consuming remaining gas despite sufficient gas for the failing opcode.
- Zero-padding of truncated PUSH immediates.
- GAS charging before observing remaining gas.
- Repeated GAS execution.
- Separation of semantic failures from implementation-specific error diagnostics.

For successful execution, compare final stack contents and remaining frame gas.

For exceptional execution, compare termination semantics and remaining frame gas. Preserve error classifications for diagnostic purposes without requiring identical exception names or partially mutated stacks across implementations.

## Limitations

- Synthetic block and transaction environments.
- Empty initial Ethereum state.
- No transaction validation or intrinsic gas accounting.
- No historical block replay.
- No validated coverage of nested calls or contract creation.
- No established conformance coverage for state-dependent opcodes.
- The reported pre-state root is not a post-execution state commitment.
- The runner is a research utility, not a production execution engine or comprehensive conformance framework.

## Conclusion

Direct Cancun EELS frame execution has been successfully validated for both normal execution and exceptional halts.

The experiment establishes a reproducible, independent reference for Ferreth's future gas-metering and opcode conformance work.

The next research step is to establish an equivalent direct execution path through the pinned REVM interpreter.

This experiment does not approve or modify ADR-002.

## Related Documentation

- [Independent EVM Conformance Research](../../docs/research/ferreth-evm-conformance.md)
- [ADR-001 — EVM Guest State ABI](../../docs/adr/001-evm-guest-state-abi.md)
- [ADR-002 — Cancun EVM Gas Metering](../../docs/adr/002-cancun-evm-gas-metering.md)
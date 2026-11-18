# REVM Cancun — Direct Frame Execution Research

## Objective

Establish a reproducible, frame-level execution reference using the REVM interpreter, independent of transaction processing and intrinsic gas accounting.

This experiment complements the [EELS Cancun research](../eels-cancun/README.md) by executing equivalent bytecode programs with identical initial frame gas budgets.

The primary observations are:

- Execution success or exceptional halt.
- Raw remaining and consumed frame gas.
- Final stack.
- Program counter.
- Instruction result and error classification.

This research supports Ferreth's incremental EVM conformance roadmap and the future implementation of ADR-002.

## Environment

- Ethereum fork: Cancun
- Language: Rust
- REVM: 43.0.3
- revm-interpreter: 43.0.2
- revm-handler: 43.0.3
- Experiment date: 2026-11-18

REVM is configured with:

```toml
[dependencies]
revm = { version = "=43.0.3", default-features = false, features = ["std"] }
```

The experimental crate belongs to the Ferreth Cargo workspace and shares its root `Cargo.lock`.

It is excluded from the workspace's default members to keep ordinary compiler development independent of this research utility.

## Implementation

The runner constructs a REVM interpreter directly using:

- `Bytecode::new_raw()`
- `ExtBytecode`
- `SharedMemory`
- `InputsImpl`
- `EthInterpreter`
- `DummyHost`
- `SpecId::CANCUN`

Both the interpreter and the host are initialized with the Cancun specification.

The instruction and gas tables are constructed using:

```rust
let instructions =
    instruction_table::<EthInterpreter, DummyHost>();

let gas_table = gas_table_spec(SpecId::CANCUN);
```

Execution is performed through:

```rust
let action = interpreter.run_plain(
    &instructions,
    &gas_table,
    &mut host,
);
```

This bypasses transaction validation, intrinsic gas accounting, and the full REVM handler pipeline.

The runner preserves raw interpreter observations without applying exceptional-halt gas normalization.

## Setup

From the Ferreth repository root:

```bash
cargo build --locked -p revm-frame
```

## Usage

```bash
cargo run -q -p revm-frame -- --code 600260030100 --gas 20
```

| Argument | Description |
|---|---|
| `--code` | Hexadecimal EVM bytecode, with optional `0x` prefix. |
| `--gas` | Initial execution frame gas, represented as a `u64`. |

Each invocation initializes a fresh interpreter, empty stack, empty memory, and a synthetic host.

### Example output

```text
Bytecode: 600260030100
Initial Msg gas: 20
Raw remaining gas: 11
Raw gas consumed: 9
Stack: [5]
PC: 6
Success: true
Result: Stop
Error: None
```

## Experimental Results

All results in this section were observed through actual execution on 2026-11-18.

Stack values are represented from bottom to top.

All gas values reported by the runner are raw interpreter observations, not necessarily settled frame outcomes.

### 1. Successful execution

```bash
cargo run -q -p revm-frame -- --code 00 --gas 0
cargo run -q -p revm-frame -- --code 600200 --gas 20
cargo run -q -p revm-frame -- --code 600260030100 --gas 20
cargo run -q -p revm-frame -- --code 600260030300 --gas 20
cargo run -q -p revm-frame -- --code 60025000 --gas 10
cargo run -q -p revm-frame -- --code 60028000 --gas 10
```

| Program | Bytecode | Initial gas | Remaining gas | Consumed | Stack | PC |
|---|---|---:|---:|---:|---|---:|
| STOP | `00` | 0 | 0 | 0 | `[]` | 1 |
| PUSH1 | `600200` | 20 | 17 | 3 | `[2]` | 3 |
| ADD | `600260030100` | 20 | 11 | 9 | `[5]` | 6 |
| SUB | `600260030300` | 20 | 11 | 9 | `[1]` | 6 |
| POP | `60025000` | 10 | 5 | 5 | `[]` | 4 |
| DUP1 | `60028000` | 10 | 4 | 6 | `[2, 2]` | 4 |

All six executions completed successfully with `InstructionResult::Stop`.

The final stack, remaining gas, and terminal PC matched the corresponding EELS observations.

### 2. Exceptional halts and error precedence

```bash
cargo run -q -p revm-frame -- --code 01 --gas 0
cargo run -q -p revm-frame -- --code 01 --gas 2
cargo run -q -p revm-frame -- --code 01 --gas 3
cargo run -q -p revm-frame -- --code 80 --gas 3
cargo run -q -p revm-frame -- --code 600260030100 --gas 8
cargo run -q -p revm-frame -- --code 60020100 --gas 20
```

| Bytecode | Initial gas | Raw remaining | Raw consumed | Stack | PC | Result |
|---|---:|---:|---:|---|---:|---|
| `01` | 0 | 0 | 0 | `[]` | 1 | OutOfGas |
| `01` | 2 | 0 | 2 | `[]` | 1 | OutOfGas |
| `01` | 3 | 0 | 3 | `[]` | 1 | StackUnderflow |
| `80` | 3 | 0 | 3 | `[]` | 1 | StackOverflow |
| `600260030100` | 8 | 0 | 8 | `[2, 3]` | 5 | OutOfGas |
| `60020100` | 20 | 14 | 6 | `[2]` | 3 | StackUnderflow |

All six executions terminated exceptionally.

The raw interpreter retains remaining gas in the final stack-underflow case. This is investigated separately below.

### 3. Truncated PUSH1

```bash
cargo run -q -p revm-frame -- --code 60 --gas 3
```

Observed result:

```text
Bytecode: 60
Initial Msg gas: 3
Raw remaining gas: 0
Raw gas consumed: 3
Stack: [0]
PC: 3
Success: true
Result: Stop
Error: None
```

REVM interprets the missing PUSH1 immediate as zero and completes successfully.

This agrees with EELS in successful stack contents and gas consumption.

However, the terminal PC differs:

| Implementation | Stack | Remaining gas | PC |
|---|---|---:|---:|
| EELS | `[0]` | 0 | 2 |
| REVM | `[0]` | 0 | 3 |

The different terminal PC may result from REVM's analyzed bytecode padding and implicit STOP execution. The precise explanation has not been independently verified in this experiment.

Both implementations confirm that Ferreth's existing truncated-PUSH1 rejection requires a conformance correction.

### 4. GAS opcode

```bash
cargo run -q -p revm-frame -- --code 5a00 --gas 10
cargo run -q -p revm-frame -- --code 5a5a00 --gas 10
cargo run -q -p revm-frame -- --code 5a00 --gas 2
cargo run -q -p revm-frame -- --code 5a00 --gas 1
cargo run -q -p revm-frame -- --code 5a00 --gas 0
```

| Bytecode | Initial gas | Raw remaining | Raw consumed | Stack | PC | Result |
|---|---:|---:|---:|---|---:|---|
| `5a00` | 10 | 8 | 2 | `[8]` | 2 | Stop |
| `5a5a00` | 10 | 6 | 4 | `[8, 6]` | 3 | Stop |
| `5a00` | 2 | 0 | 2 | `[0]` | 2 | Stop |
| `5a00` | 1 | 0 | 1 | `[]` | 1 | OutOfGas |
| `5a00` | 0 | 0 | 0 | `[]` | 1 | OutOfGas |

REVM confirms the GAS behavior previously observed in EELS:

- GAS costs two gas units.
- GAS charges its own cost before pushing the remaining gas.
- Repeated GAS instructions observe the decreasing frame budget.
- A frame with exactly two gas units can execute GAS successfully.
- A frame with fewer than two gas units terminates exceptionally.

## EELS vs. REVM — Differential Observations

### Experimental summary

| Metric | Result |
|---|---:|
| Total vectors | 18 |
| Successful executions | 10 |
| Exceptional halts | 8 |
| Successful stack matches | 10/10 |
| Successful remaining gas matches | 10/10 |
| Successful terminal PC matches | 9/10 |
| Termination classification matches | 18/18 |
| Exceptional error classification differences | 3 |
| Exceptional raw gas discrepancies | 1 |
| Exceptional stack discrepancies | 2 |

Termination classification refers to successful execution versus exceptional halt, not exact error enum equality.

### 1. Error precedence

EELS and REVM differ in the order in which ADD validates stack operands and charges static gas.

REVM's `Interpreter::step()` performs the following operations:

1. Read the opcode.
2. Advance the instruction pointer.
3. Charge static opcode gas.
4. Dispatch the instruction handler.

EELS ADD extracts its stack operands before charging its ordinary opcode gas.

The difference is observable when executing ADD with an empty stack:

| Initial gas | EELS | REVM |
|---:|---|---|
| 0 | StackUnderflowError | OutOfGas |
| 2 | StackUnderflowError | OutOfGas |
| 3 | StackUnderflowError | StackUnderflow |

All three cases terminate exceptionally in both implementations.

Ferreth should define a deterministic error-precedence policy without requiring agreement with incompatible implementation-specific diagnostics.

### 2. Empty-stack DUP1

For bytecode `0x80` with initial gas `3`:

| Implementation | Result |
|---|---|
| EELS | StackUnderflowError |
| REVM | StackOverflow |

Both implementations terminate exceptionally with zero remaining gas.

This confirms the diagnostic classification discrepancy previously observed through Ferreth's transaction-based REVM conformance tests.

The difference alone does not establish a consensus-semantic mismatch.

### 3. Exceptional stack mutations

| Bytecode | Initial gas | EELS stack | REVM stack |
|---|---:|---|---|
| `60020100` | 20 | `[]` | `[2]` |
| `600260030100` | 8 | `[]` | `[2, 3]` |

EELS may extract operands before detecting an exceptional condition.

REVM preserves the operands in these observed cases.

Terminal stack contents after exceptional execution are implementation diagnostics and should not be required to match across independent execution engines.

### 4. Program counter differences

| Bytecode | Initial gas | EELS PC | REVM PC |
|---|---:|---:|---:|
| `01` | 0 | 0 | 1 |
| `01` | 2 | 0 | 1 |
| `01` | 3 | 0 | 1 |
| `80` | 3 | 0 | 1 |
| `600260030100` | 8 | 4 | 5 |
| `60020100` | 20 | 2 | 3 |
| `5a00` | 1 | 0 | 1 |
| `5a00` | 0 | 0 | 1 |
| `60` | 3 | 2 | 3 |

The exceptional PC differences are consistent with REVM advancing its instruction pointer before charging static gas and dispatching the instruction.

Terminal PC values should be preserved for diagnostic purposes, but should not be treated as a universal conformance criterion.

## Exceptional-Halt Gas Settlement

### Experimental evidence

The most significant raw gas discrepancy was observed using:

```bash
cargo run -q -p revm-frame -- --code 60020100 --gas 20
```

Program:

```text
PUSH1 2
ADD
STOP
```

Observed REVM output:

```text
Bytecode: 60020100
Initial Msg gas: 20
Raw remaining gas: 14
Raw gas consumed: 6
Stack: [2]
PC: 3
Success: false
Result: StackUnderflow
Error: StackUnderflow
```

Comparison:

| Observation | EELS | Direct REVM |
|---|---:|---:|
| Initial gas | 20 | 20 |
| Raw remaining gas | 0 | 14 |
| Raw consumed gas | 20 | 6 |
| Terminal stack | `[]` | `[2]` |
| Terminal PC | 2 | 3 |
| Error | StackUnderflowError | StackUnderflow |

This difference prompted an inspection of REVM's interpreter and handler implementations.

### Source evidence

#### Interpreter-level execution

Source: `revm-interpreter` 43.0.2, `src/interpreter.rs`.

Function: `Interpreter::halt()`.

```rust
pub fn halt(&mut self, result: InstructionResult) {
    if result == InstructionResult::OutOfGas {
        self.gas.spend_all();
    }

    self.bytecode
        .set_action(InterpreterAction::new_halt(result, self.gas));
}
```

The interpreter explicitly consumes all remaining gas for OutOfGas.

Other exceptional results may preserve residual gas in the raw interpreter result.

#### Child-frame gas settlement

Source: `revm-handler` 43.0.3, `src/frame.rs`.

Function: `handle_reservoir_remaining_gas()`.

Relevant excerpts:

```rust
if instruction_result.is_halt() {
    child_gas.spend_all();
}
```

```rust
if instruction_result.is_ok_or_revert() {
    parent_gas.erase_cost(child_gas.remaining());
}
```

Exceptional halts consume the child's remaining regular gas.

Unused regular gas returns to the parent only on successful execution or revert.

#### Root-frame gas settlement

Source: `revm-handler` 43.0.3, `src/handler.rs`.

Function: `last_frame_result()`.

The handler settles the initial execution frame into transaction-level gas using the same settlement function:

```rust
handle_reservoir_remaining_gas(
    instruction_result,
    parent_gas,
    frame_result.gas_mut().tracker_mut(),
);
```

This establishes the exceptional-halt settlement behavior at the source level for both child and initial frames.

The complete handler path was not executed by this research runner.

### Interpretation

EELS `process_message()` and REVM `run_plain()` expose different execution boundaries.

EELS includes exceptional-halt gas consumption in its returned frame state.

REVM separates raw interpreter execution from subsequent handler-level gas settlement.

Consequently, the residual 14 gas units observed in the failing ADD experiment are not evidence that REVM preserves that gas in the settled frame outcome.

The expected settled remaining gas is zero, based on the inspected handler source.

### Normalization policy

For the tested Cancun execution model, a future differential comparison can apply the following policy to REVM's raw interpreter result:

```rust
let normalized_remaining_gas = if result.is_halt() {
    0
} else {
    result.gas.remaining()
};
```

This normalization is intended specifically for exceptional halts.

It must not be applied indiscriminately to every unsuccessful result, because REVERT preserves unused frame gas.

The current research runner deliberately does not perform this normalization.

### Later-fork considerations

The inspected REVM handler contains state-gas and reservoir accounting associated with later Ethereum proposals, including EIP-8037 and EIP-2780.

Those mechanisms are outside the scope of this Cancun experiment.

Only the relevant regular-gas settlement behavior is considered here.

## Conformance Implications

The combined EELS and REVM experiments support the following comparison policy:

| Observation | Successful execution | Exceptional halt |
|---|---|---|
| Termination classification | Required | Required |
| Settled remaining gas | Required | Required |
| Final stack | Required | Diagnostic |
| Exact error classification | Not applicable | Diagnostic |
| Terminal PC | Diagnostic | Diagnostic |
| Raw interpreter gas | Useful | Diagnostic |

These observations establish concrete requirements for Ferreth's future conformance tests:

- Exact successful opcode gas consumption.
- Correct execution at exact gas boundaries.
- Zero-cost STOP.
- Truncated PUSH immediate zero-padding.
- GAS charging before reading the remaining budget.
- Repeated GAS execution.
- Zero settled remaining gas after an exceptional halt.
- Explicit and deterministic error-precedence policy.
- Separation of semantic failures from implementation-specific diagnostics.

### Required regression case

Following ADR-002 implementation, Ferreth should include a regression test for:

```text
Bytecode: 60020100
Initial frame gas: 20
Expected termination: Exceptional halt
Expected remaining frame gas: 0
Expected diagnostic: StackUnderflow
```

Proposed test name:

```text
stack_underflow_consumes_remaining_frame_gas
```

The exceptional terminal stack does not need to reproduce either reference implementation's internal mutations.

The architectural responsibility for exceptional gas settlement must be established during ADR-002 review.

## Limitations

- Execution is limited to the direct interpreter path.
- No transaction validation or intrinsic gas accounting is performed.
- `DummyHost` supplies synthetic default values rather than a real Ethereum state.
- State-dependent opcode behavior is not validated.
- Nested calls and contract creation are not executed through the complete frame handler.
- Non-return interpreter actions are not supported by the runner.
- REVERT behavior was inspected conceptually and at the handler source level, but was not tested experimentally.
- Full handler-level exceptional gas settlement was verified through source inspection, not through direct execution of the handler.
- Later-fork state-gas and reservoir semantics are outside the scope of this research.
- The experimental dataset is small and does not establish comprehensive Cancun conformance.

The runner is a research utility, not a production execution engine or a replacement for Ferreth's existing differential test suite.

## Conclusion

Direct REVM Cancun frame execution has been successfully validated.

Across 18 experimental vectors, EELS and REVM agree on successful versus exceptional termination. All ten successful executions produce identical final stacks and remaining gas.

Observed differences in error classification, exceptional stack mutations, terminal PC, and raw gas accounting are attributable to implementation-specific diagnostics or execution boundaries.

Raw exceptional gas accounting must be distinguished from settled frame gas before using REVM as a reference for Ferreth's gas-metering implementation.

Together with the EELS Cancun runner, this experiment provides an independent and reproducible foundation for Ferreth's incremental EVM conformance work.

The next research priority is to address the identified conformance discrepancies before approving and implementing ADR-002.

This experiment does not modify or approve ADR-002.

## Source References

- [revm 43.0.3](https://docs.rs/crate/revm/43.0.3/source/)
- [revm-interpreter 43.0.2 — interpreter.rs](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter.rs)
- [revm-interpreter 43.0.2 — instructions.rs](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/instructions.rs)
- [revm-context-interface 43.0.2 — host.rs](https://docs.rs/crate/revm-context-interface/43.0.2/source/src/host.rs)
- [revm-handler 43.0.3 — frame.rs](https://docs.rs/crate/revm-handler/43.0.3/source/src/frame.rs)
- [revm-handler 43.0.3 — handler.rs](https://docs.rs/crate/revm-handler/43.0.3/source/src/handler.rs)

## Related Documentation

- [EELS Cancun — Frame Execution Research](../eels-cancun/README.md)
- [Independent EVM Conformance Research](../../docs/research/ferreth-evm-conformance.md)
- [ADR-001 — EVM Guest State ABI](../../docs/adr/001-evm-guest-state-abi.md)
- [ADR-002 — Cancun EVM Gas Metering](../../docs/adr/002-cancun-evm-gas-metering.md)
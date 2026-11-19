# ADR-002: Cancun EVM Gas Metering

- **Status:** Proposed
- **Date:** 2026-11-17
- **Target:** Ethereum Cancun (Dencun execution fork); REVM 43.0.3 with `SpecId::CANCUN` is the project's differential oracle.
- **Implementation:** Not implemented; this proposal authorizes no code changes.

## 1. Context

Ferreth compiles a supported bytecode program into one linked RV64 guest. Under accepted [ADR-001](001-evm-guest-state-abi.md), guest code owns the live EVM stack in memory; `s0`/`s1` are its base and next-free pointers; Rust initializes and executes the guest without a callback per opcode; and deliberate normal or guest-exception paths publish a result before transferring to the executor-observed PC sentinel. RV64 instruction counts are implementation metrics, not EVM gas.

The current compiler supports `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, and `DUP1`; empty bytecode also completes normally by fallthrough. The decoder's wider PUSH recognition does not imply compiler support. The 24-byte metadata record contains status, exception code, and final `s1`, but no gas. Existing REVM differential tests use Cancun for stack and terminal outcomes, not gas.

This ADR proposes gas accounting for an execution-frame budget. It does not add transaction intrinsic gas, storage, memory opcodes, calls, or a transaction API.

## 2. Existing constraints and verified Cancun behavior

Established by ADR-001 and source inspection:

- Guest execution remains autonomous: no Rust opcode dispatch or host-side gas meter.
- The EVM stack is ascending guest memory, independent of native `sp`; `s0`/`s1` remain reserved. Existing ADD/SUB cores use `a0`–`a2` and `t0`–`t5` and preserve `s0`/`s1`.
- Normal and exceptional publishers write final `s1`, exception code, then terminal status, and jump to the sentinel. The executor observes the sentinel without executing its slot. Executor faults and step-limit exhaustion remain host errors.
- The initial 4-KiB metadata region has ample unused bytes, but the accepted record is exactly 24 bytes. Physical room does not itself authorize changing its ABI.
- Opcode generators are linked bodies with patchable exceptional branches. There is no common gas prologue today.
- REVM 43.0.3 resolves `revm-interpreter` 43.0.2 (Cargo.lock); its `Interpreter::step` charges the static gas-table entry before executing an opcode handler. Stack handlers then validate underflow/overflow. The interpreter gas counter debits only if sufficient gas exists.

For the initial operations, the Cancun static costs are:

| Opcode | Cost |
|---|---:|
| STOP | 0 |
| ADD, SUB, PUSH1, DUP1 | 3 each |
| POP | 2 |
| GAS (0x5a) | 2 |

REVM's Cancun gas table confirms these values. The Cancun execution specifications define opcode steps as gas charge followed by operation; for stack instructions this means a stack failure at a budget sufficient for the opcode's cost is reported as the stack error, while a budget below cost fails as OOG first. Equality succeeds. Since any exceptional halt consumes the frame's remaining gas, terminal remaining gas is zero for OOG and stack exceptions; that does not alter the cost/ordering observed before the halt.

`GAS` is charged 2 before its handler pushes the resulting remaining gas. It must validate stack capacity after the charge as other pushes do. Therefore with initial execution gas 10, `GAS; GAS; STOP` pushes 8 then 6, leaves stack `[8, 6]` (bottom to top), and terminates with 6 remaining: total execution gas used is 4. If less than 2 gas remains at GAS, it OOGs without pushing; if the stack is full and at least 2 gas is available, it charges 2 and stack-overflows. No partial push occurs on either failure.

The exceptional-gas rule is distinct from normal live gas: successful STOP and implicit fallthrough preserve unused gas; exceptional halt reports zero remaining, even when a stack exception occurred after charging less than the full budget. Gas already consumed is never restored by rollback. Any future reversible EVM state must follow its own Cancun frame rollback lifecycle, independently of gas accounting.

## 3. Proposed architecture and decisions for review

These are recommendations, not accepted decisions.

1. **Scope and ownership.** Treat the input as a `u64` execution-frame gas limit in gas units. The guest is authoritative for mutable remaining gas and the terminal gas result. The host initializes the budget and validates/reports the guest publication; it does not meter or repair gas. Transaction intrinsic gas remains outside this execution API.
2. **State and result responsibilities.** Keep mutable execution state (at least remaining gas) conceptually separate from the terminal result (status, stable exception category, final stack pointer, terminal gas accounting). They may share the existing metadata allocation physically. Preserve ADR-001 publication ordering: publish final stack pointer and all result fields before terminal status, then use the existing sentinel path.
3. **Smallest ABI extension.** Extend the current metadata record in place by appending gas fields in unused metadata memory, keeping existing offsets and status values unchanged. Initially represent the input limit and published terminal remaining gas as `u64`; derive gross used as `limit - remaining`, with checked validation. Exceptional outcomes publish remaining zero; successful outcomes publish actual remaining gas. No redundant persistent consumed-gas field is needed. Assign stable OOG and stack exception values while preserving existing provisional stack codes until an explicit ABI review decides whether renumbering is required. This is an ABI extension requiring maintainer approval; it does not reinterpret ADR-001 or require modifying ADR-001's text.
4. **One semantic charging contract.** Each opcode supplies its Cancun cost (or computes a complete dynamic cost in a later milestone) to the same semantic operation: validate sufficient remaining gas without underflow; on insufficiency, take the guest OOG publisher without opcode effects; otherwise debit exactly once, then perform the operation's Cancun-defined stack and state work. Stack checks follow charge when REVM/spec order does so. The charge contract specifies semantics, not a code-generation mechanism.
5. **Initial implementation technique.** Prefer straightforward inline RV64 sequences in the existing opcode bodies for the seven-opcode milestone. Reuse the current exceptional publication mechanism, adding OOG classification. Do not add a general runtime framework or helper-call ABI for seven fixed costs. A shared guest routine can be reconsidered if measurements or dynamic-cost implementation show duplicated code is costly and a clear register/stack convention exists.
6. **State and rollback boundary.** A failed gas check has no opcode side effects. More generally, gas consumption and reversible EVM state have independent lifecycles: reverted/failed frames do not regain spent gas, while state is committed or rolled back under Cancun rules. Journals, nested frames, and state representation are deferred.

All guest paths must retain `s0`/`s1`, avoid partial stack writes on a failed charge, and publish a distinguishable OOG terminal result through ADR-001's deliberate exit path. RV64 traps are not EVM OOG.

## 4. Alternatives and tradeoffs

- **Host-managed metering / opcode callbacks:** rejected for this compiler model. It requires observing every executed opcode and conflicts with the single autonomous guest program and callback-free executor contract.
- **Per-opcode guest checks:** recommended correctness baseline. It gives exact failure location and exposes remaining gas to GAS, at the cost of extra guest instructions.
- **Basic-block aggregation:** defer. It complicates exact failure points and cannot preserve GAS observations or exceptional ordering without splitting back to opcode boundaries. Consider only after a correct per-opcode baseline and equivalence tests.
- **Register-cached gas or shared helper:** defer as implementation optimizations. No gas register is reserved, and a helper adds calling convention constraints. Measure code size/instruction overhead before changing the simpler inline baseline.
- **Separate result ABI/versioned record:** possible if in-place extension proves incompatible with an external consumer. Current evidence shows unused space in the existing metadata allocation and no deployed compatibility reader; appending fields while retaining the first 24 bytes is the smallest candidate.

## 5. Initial milestone and conformance

The initial functional milestone MUST compile and meter `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, `DUP1`, and `GAS`. `GAS` is required now for semantic completeness and guest-side observation; it is not deferred to dynamic gas support.

Differential tests must give Ferreth and REVM equivalent execution-frame budgets, initial stack, bytecode, fork, and execution context. The existing REVM transaction harness currently uses a transaction gas limit; the test harness must explicitly normalize or expose a frame-equivalent budget before gas comparisons. Compare:

- Cost and below/equal/above-cost boundaries for every opcode, including zero-cost STOP.
- Multi-opcode programs, explicit STOP, and implicit fallthrough.
- Intermediate GAS values, including repeated GAS. For budget 10, `GAS; GAS; STOP` must yield `[8, 6]`, remaining 6, used 4.
- OOG versus underflow/overflow precedence, zero/exact boundaries, exceptional remaining gas, and no failed-op partial stack effects.
- Final stack, status/exception category, guest-published gas, and host-validated gas arithmetic.

Maintain two metric families:

- **EVM semantic:** execution budget, gross execution gas used, remaining gas, terminal status.
- **RV64 efficiency:** executed guest instructions, generated code size, and metering overhead.

Conformance is the acceptance criterion. Performance is informational until correctness is established; the current benchmark infrastructure can later record instruction counts and code size, but no benchmark schema or overhead target blocks the first working guest.

## 6. Questions and deferrals

### Correctness blocker before implementation

1. **Frame-equivalent REVM oracle setup.** Existing conformance invokes REVM through transaction execution and has no gas budget parameter in Ferreth's guest API. Before coding comparisons, add or derive a focused test path that runs the Cancun interpreter with an explicit frame budget, or prove the transaction wrapper's exact intrinsic deduction and returned frame gas for the restricted test context. Verify STOP, OOG, stack error, and repeated GAS cases against that path. This is an experiment/test-harness task; it does not block maintainer review of the proposed contracts.

### Decisions safely deferred

- **Inline sequence vs helper and register caching:** choose inline first; reconsider only after code-size/instruction measurements or when multiple dynamic-cost opcodes need common logic.
- **Basic-block metering:** revisit only if a correct per-opcode implementation exists and an equivalence strategy proves identical GAS values, failing opcode, side effects, and terminal result.
- **Dynamic cost helpers, memory expansion, warm/cold access, refunds, transaction context, nested frames:** define only the invariant now: preflight the complete charge before effects; preserve independent gas and reversible-state lifecycles. Design concrete structures when an opcode requires them.
- **Gas width beyond `u64` and separate gross-used field:** current API is execution gas and REVM uses `u64`; use checked arithmetic and reject out-of-range inputs. Revisit if an upstream interface or execution scope requires a wider value. Derive used from budget and remaining to avoid redundant fields.
- **Benchmark report schema:** defer until measured overhead exists; add EVM and RV64 fields in separate namespaces when needed.
- **Normative execution-spec commit:** use the Cancun fork documentation and pinned REVM as the current review corpus. Pin an execution-spec repository revision if implementation review finds a semantic ambiguity or reproducibility requires source-level archival.

## 7. Consequences

This proposal preserves guest autonomy, stack ownership, register conventions, and terminal control flow. It requires a reviewed extension of the published metadata ABI but retains its current 24-byte prefix and publication protocol. Inline per-opcode checks will increase RV64 code and instruction counts; this is acceptable for the correctness baseline. OOG and stack errors become distinct guest results, and exceptional gas reports zero remaining. Dynamic EVM features remain unsupported until their required state and rollback behavior are designed and tested.

## 8. References and investigation evidence

### Repository evidence

- [ADR-001](001-evm-guest-state-abi.md), §§3–8: stack/register contract, memory regions, metadata publication, and sentinel termination.
- `crates/ferreth-compiler/src/compiler.rs`: compiler subset, opcode linker, normal and exceptional publishers.
- `crates/ferreth-compiler/src/rv64/{guest.rs,memory.rs}`: 24-byte metadata, stack and memory layout.
- `crates/ferreth-compiler/src/rv64/{push.rs,add.rs,sub.rs,pop.rs,dup.rs}`: stack-aware generators and exception branches.
- `crates/ferreth-compiler/src/orchestrator.rs`, `rv64/executor/{mod.rs,rvemu.rs}`: guest initialization, execution boundary, result reading, sentinel and metrics.
- `crates/ferreth-conformance/src/lib.rs`, `crates/ferreth-conformance/tests/rc_revm_conformance.rs`: REVM Cancun execution and existing differential harness.
- `Cargo.lock`: REVM 43.0.3 and resolved `revm-interpreter` 43.0.2.
- `crates/ferreth-conformance/src/benchmark.rs`, `docs/runbooks/benchmark.md`: RV64 instruction metrics only.

### Cancun and REVM sources

- [Ethereum execution-specifications, Cancun](https://ethereum.github.io/execution-specs/src/ethereum/forks/cancun/), including the Cancun VM instruction definitions and exceptional-halt model.
- [Cancun stack instruction source](https://ethereum.github.io/execution-specs/src/ethereum/forks/cancun/vm/instructions/stack.html) and [Cancun system instruction source](https://ethereum.github.io/execution-specs/src/ethereum/forks/cancun/vm/instructions/system.html): charging order and GAS behavior.
- [Cancun exceptions](https://ethereum.github.io/execution-specs/src/ethereum/forks/cancun/vm/exceptions.html): exceptional halt consumes remaining frame gas; REVERT is distinct.
- Pinned local source: `revm-interpreter 43.0.2`, `src/interpreter.rs::Interpreter::step`, `src/instructions.rs::gas_table_impl`, `src/instructions/{stack.rs,system.rs,macros.rs}`. Cargo.lock resolves this as part of REVM 43.0.3.
- EIP-2200, EIP-2929, and EIP-3529 are future storage/refund references, not part of this milestone.

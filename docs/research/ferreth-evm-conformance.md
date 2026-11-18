# Independent EVM Conformance Research for Ferreth

**Research snapshot:** 17 November 2026

**Fork under test:** Cancun

**Status:** Non-normative research with verification records through Priority 3

**Scope:** Ferreth's six implemented opcodes (`STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, `DUP1`) and possible `GAS` support.

## Executive summary

Priorities 1 and 2 established executable Cancun frame references in [EELS](../../research/eels-cancun/README.md) and [REVM](../../research/revm-frame/README.md). Priority 3 corrected truncated PUSH decoding and updated the existing transaction-based differential tests to compare termination and successful stacks. The independent frame runs provide evidence for future gas work.

The original research contained several important overstatements:

1. **EEST did move.** The `ethereum/execution-spec-tests` repository was archived read-only on 2 July 2026. Its README says code was migrated to `ethereum/execution-specs` and fixture releases are now published there. This is a repository and release move, not evidence that every old fixture family has already been migrated.
2. **EELS `process_message` requires a message environment.** At the pinned `execution-specs` revision it is `process_message(message: Message) -> Evm`, in the Cancun interpreter module. It seeds an empty stack and the message's frame gas, then returns the internal EVM object. The [Priority 1 runner](../../research/eels-cancun/README.md) constructs the required environment and transaction state and records end-to-end results.
3. **Stack-operation ordering differs between EELS and REVM.** Cancun EELS often pops operands before charging (e.g. ADD, SUB, POP), while REVM's interpreter charges the static opcode gas before dispatching its handler. Both treat exceptional halt as a failed frame; internal error labels and partial stack observations can differ. These differences are not, by themselves, externally observable consensus disagreements.
4. **The known empty-stack DUP1 label difference reproduces.** Ferreth's existing REVM 43 integration test reports REVM `StackOverflow` and guest `StackUnderflow`. Cancun EELS checks the missing DUP operand and raises `StackUnderflowError`. REVM's DUP handler maps a failed stack duplication to `StackOverflow`. This is a diagnostic taxonomy mismatch; do not make enum-name equality the conformance criterion.
5. **Ferreth zero-pads truncated PUSH immediates.** The decoder applies this rule to PUSH1–PUSH32, and the RV64 compiler lowers PUSH1. The REVM differential tests execute through a transaction with a 1,000,000 gas limit and compare termination and successful stacks.
6. **Historical VMTests need a provenance caveat.** The `ethereum/tests` repository is not archived, but the old VM Tests wiki is from 2015, and the current `develop` branch does not expose the historical top-level `VMTests`/`VMTest` paths. The wiki describes execution gas and remaining gas, but not an initial or expected final stack. No specific current JSON fixture was verified.

These are research findings, not changes to [ADR-001](../adr/001-evm-guest-state-abi.md) or [ADR-002](../adr/002-cancun-evm-gas-metering.md). ADR-002 remains **Proposed** and its frame-budget / error-precedence decisions need review against the source-order evidence below.

## 1. Repository-grounded Ferreth context

Repository observations:

- [compiler.rs](../../crates/ferreth-compiler/src/compiler.rs) compiles `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, and `DUP1`. It stops decoding at `STOP`. Reaching the end of bytecode completes normally. The decoder recognizes PUSH1–PUSH32, but the compiler rejects all PUSH widths other than PUSH1.
- The [decoder](../../crates/ferreth-compiler/src/lib.rs) right-pads truncated PUSH immediates with zeros and retains `DecodeError::TruncatedPush` for source compatibility. The [static disassembler](../../crates/ferreth-compiler/src/disassembler.rs) reports the physical bytes and the number of missing bytes.
- [dup.rs](../../crates/ferreth-compiler/src/rv64/opcodes/dup.rs) checks underflow before capacity and leaves the stack unchanged on either error. [guest.rs](../../crates/ferreth-compiler/src/rv64/guest.rs) defines the 1024-word stack and its guest-side stack validation.
- [rc_revm_conformance.rs](../../crates/ferreth-compiler/tests/rc_revm_conformance.rs) runs REVM with `SpecId::CANCUN`, through `inspect_tx`, with transaction gas limit 1,000,000. Its inspector captures the interpreter stack after instructions. It contains an explicit test for the empty-stack DUP1 diagnostic mismatch.
- [execution.rs](../../crates/ferreth-compiler/src/execution.rs) also uses Cancun REVM, but its transaction path returns call output rather than frame stack/gas.
- [Cargo.lock](../../Cargo.lock) resolves `revm` 43.0.3 and `revm-interpreter` 43.0.2. The compiler crate depends directly on `revm` 43.0.3; it does not declare a separate direct `revm-interpreter` dependency.
- [benchmark.rs](../../crates/ferreth-compiler/src/benchmark.rs) measures RV64 instruction counts, not EVM gas.

The current guest ABI has status, provisional exception code and final stack pointer; gas is not part of the ABI. GAS and gas metering are not implemented. ADR-001 is Accepted; ADR-002 is Proposed. This report does not authorize implementation or change either ADR.

The gas-order observations in RF-003 remain relevant to review of ADR-002's proposed ordering of gas checks and stack validation.

## 2. Cancun EELS frame feasibility

### Source and API

The inspected source is from `ethereum/execution-specs` commit [`d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3`](https://github.com/ethereum/execution-specs/commit/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3), the repository head observed on 17 November 2026. The Cancun-specific files remain under `src/ethereum/forks/cancun/`; do not follow a latest-fork path and assume it means Cancun.

In [`vm/interpreter.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/interpreter.py), the function is:

```python
process_message(message: Message) -> Evm
```

It is a source-level execution function, not a convenience API accepting `(bytecode, gas)`. `Message` is defined in [`vm/__init__.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/__init__.py) and contains block and transaction environments, caller/target, bytecode, frame gas, value/data, depth, static/value-transfer flags, access sets and parent-frame reference. `BlockEnvironment` and `TransactionEnvironment` have required fields; `TransactionEnvironment` owns a `TransactionState`. `TransactionState` in [`state_tracker.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/state_tracker.py) in turn refers to a `BlockState` and pre-state provider.

The relevant frame behavior is visible in the function body:

- `Evm.stack` starts as `[]`; `Evm.gas_left` starts at `message.gas`; bytecode is `message.code`.
- It snapshots transaction state even for simple arithmetic. The [Priority 1 runner](../../research/eels-cancun/README.md) supplies a synthetic environment and empty pre-state.
- A normal message target should be non-empty and should not select a precompile. Set `should_transfer_value=False`; stack-only opcodes do not otherwise need state reads, but `process_message` still uses the transaction state for snapshot/rollback.
- The loop runs while `evm.running` and `evm.pc < len(evm.code)`. Falling off the end is normal completion. Explicit STOP sets `running=False`.
- The returned `Evm` exposes `stack`, `gas_left`, `error`, `pc`, `output` and other frame fields. `process_message_call` instead returns `MessageCallOutput` without the final stack; it is the wrong return type for this comparison.
- `ExceptionalHalt` is caught, sets `gas_left=0`, clears output and stores the error. `Revert` has a separate non-zeroing path. The Python exception subtype is an implementation diagnostic, not automatically a consensus-visible output.

### Opcode details and ordering

The Cancun implementation sources are [`arithmetic.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/arithmetic.py), [`stack.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/stack.py), [`gas.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/gas.py), [`control_flow.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/control_flow.py) and [`memory.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/memory.py).

| Opcode | Cancun source behavior relevant to Ferreth | Cancun static cost |
|---|---|---:|
| STOP | No stack operands; stops frame | 0 |
| PUSH1 | Charges, reads one immediate byte, pushes; truncated input is zero-padded by `buffer_read` | 3 |
| ADD / SUB | Pops two operands, then charges; `x` is the top item, so SUB computes top minus next, modulo 2^256 | 3 |
| POP | Pops before charging | 2 |
| DUP1 | Charges before checking requested depth; then duplicates and pushes | 3 |
| GAS | Charges 2, then pushes the remaining frame gas | 2 |

This order is material to diagnostic experiments, not to the final classification of a failed frame alone. For example, an empty-stack ADD at budget 0 underflows before EELS checks gas; REVM charges static gas before invoking ADD and reports OOG. Both are exceptional halts, and the terminal gas is consumed. For ADD with operands but insufficient gas, EELS pops before `charge_gas`; the returned exceptional `Evm.stack` may therefore be partially changed. Such terminal stack values are not normal externally observable EVM results and should be treated as diagnostics.

For a successful frame, `GAS; GAS; STOP` with frame budget 10 pushes 8 then 6 and finishes with 6 gas, as observed by the [EELS runner](../../research/eels-cancun/README.md#5-gas-opcode). Source inspection shows how the gas check precedes the push and how `DUP1` at capacity fails on the push after its gas charge.

### Installation and experiment status

The pinned repository `pyproject.toml` identifies the package as `ethereum-execution`, requires Python >=3.11, and lists its runtime dependencies. Its README says Python 3.11–3.14 are supported (3.12 recommended) and uses `uv sync`; the `ethereum-spec-evm` CLI is supplied by the `ethereum-execution-testing` workspace package and runs transitions/state tests, not arbitrary frames. EEST tooling is in the repository's test dependency group, not a lightweight, preconfigured Ferreth runner. A reproducible run should pin the repository SHA above, record Python and lockfile, and use the repository's supported environment tooling.

The initial source-review environment returned `ModuleNotFoundError: No module named 'ethereum'` for `python3 -c 'import ethereum; print(ethereum.__file__)'`. The later [Priority 1 experiment](../../research/eels-cancun/README.md) used a locked Python environment and recorded successful and exceptional executions.

## 3. EEST: current repository and fixture fit

The archived [`ethereum/execution-spec-tests` README](https://github.com/ethereum/execution-spec-tests/blob/main/README.md) states that the repository was archived on 2 July 2026, its code migrated to `ethereum/execution-specs`, and releases are now published from the latter. This is strong primary evidence for the move. It does not show that every legacy fixture family was migrated.

The current [`execution-specs` release `tests@v21.0.2`](https://github.com/ethereum/execution-specs/releases/tag/tests%40v21.0.2), published 26 October 2026, provides `fixtures.tar.gz` (about 939 MB). The release notes distinguish state, blockchain and transaction fixture formats. The current repository also contains fork-scoped tests, including `tests/cancun/`; its `tests/frontier/opcodes/test_all_opcodes.py` generates state tests that exercise opcode success/failure through contract calls and assert storage state. The suite is useful as a test-source reference, not a simple frame vector.

EEST fixtures are generated execution-client/state-transition tests. A fixture can encode a fork, pre-state, transaction and expected post-state, but that does not make the final internal EVM stack a portable asserted field. Ferreth currently cannot consume these in a meaningful general way without a state/transaction adapter. Inspect one pinned fixture and its exact assertions before considering any adapter; do not infer stack-level assertions from a test name such as `test_dup`.

**Classification:** repository migration and current release location are verified; EEST has test/filler infrastructure and Cancun cases; practical direct use for Ferreth's six-opcode execution frame is poor. The cited opcode suite is Frontier-origin source that can be parameterized by fork, not a Cancun-only test. No large fixture asset was downloaded or executed.

## 4. Historical Ethereum VMTests

The [`ethereum/tests` VM Tests wiki](https://github.com/ethereum/tests/wiki/VM-Tests) describes VM-only execution, excluding transaction-processing gas, and documents `exec.gas` as gas available to execution and top-level `gas` as gas remaining. It describes `env`, `pre`, `exec`, `gas`, `logs`, `out`/`output`, `post` and `callcreates` fields. The wiki is dated 2015, and its own section names vary (`out` in the structure example, `output` in prose); use the schema from the exact fixture release rather than treating the wiki as a current schema.

The wiki does **not** document arbitrary initial stack input or a final stack assertion. Expected state, gas, output and logs may be helpful only when the bytecode can surface the behavior through supported observable outputs. It says exceptions are represented by absence of the expected result sections. Its VM gas budget is conceptually closer to an EVM frame than a transaction gas limit, but does not by itself identify a Cancun fixture or expected stack.

The repository [`ethereum/tests`](https://github.com/ethereum/tests) is not archived (GitHub repository metadata reports `archived=false`); it is not correct to call it an archived repository. On its current `develop` branch, the historical top-level [`VMTests`](https://github.com/ethereum/tests/tree/develop/VMTests) and [`VMTest`](https://github.com/ethereum/tests/tree/develop/VMTest) paths are absent. An open issue in `ethereum/execution-specs` [#2556](https://github.com/ethereum/execution-specs/issues/2556) proposes archiving `ethereum/tests` and `ethereum/legacytests` after migrating remaining fillers; it explicitly presents itself as a draft needing review. That is evidence of an unfinished workstream, not evidence of completed migration or parity.

No concrete current JSON VMTest, Cancun fork annotation, or per-case generator provenance was verified in this research. The previous report's specific historical case counts and path examples are not retained as facts. Treat the wiki as a schema/history reference unless a specific release and actual JSON case can be pinned and inspected. Potential classification for a future candidate:

- **Directly usable:** only if the exact fixture's bytecode, execution gas and asserted result match Ferreth's observable surface.
- **Small adapter:** if it needs only conversion of bytecode and a frame gas limit, with no unsupported transaction/state semantics.
- **Research reference:** if it exposes no final stack, has older fork provenance, or expected values were generated by an unknown client.
- **Not suitable now:** if it depends on full state, transaction processing, call/creation behavior, memory, or unsupported opcodes.

## 5. REVM 43.0.3 and frame equivalence

### Existing harness

Ferreth's conformance test builds a Cancun REVM context and calls `inspect_tx` on a transaction with a 1,000,000 gas limit. Its `StackInspector` saves stack contents at interpreter initialization and on `step_end`. The differential checks cover termination and successful stacks. A frame-gas comparison needs a matching EVM frame budget. Transaction calldata, access lists, creation, execution setup, refunds and fork-specific accounting can affect transaction-level figures.

A carefully constrained transaction might be normalized if its envelope, intrinsic costs, call forwarding and pre-op gas are all controlled and verified. That is not what the current harness does, and no such normalization was established here. Transaction results do not publish the final EVM stack; the current inspector obtains stack observations separately.

### Low-level API found in pinned source

The `revm-interpreter` 43.0.2 source resolved by Ferreth's lockfile exposes a low-level interpreter. `Interpreter::new` takes `SharedMemory`, `ExtBytecode`, `InputsImpl`, static flag, `SpecId` and explicit `gas_limit: u64`. `Interpreter` exposes its stack and gas fields; the stack can be seeded using its stack API. `run_plain(instruction_table, gas_table, host)` returns an `InterpreterAction`, whose return variant contains an `InterpreterResult` with `InstructionResult` and gas.

The [Priority 2 runner](../../research/revm-frame/README.md) compiled and exercised this API with explicit Cancun frame gas and a synthetic host. The pinned source paths are [`interpreter.rs`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter.rs), [`interpreter_types.rs`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter_types.rs) and [`instructions/stack.rs`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/instructions/stack.rs).

REVM's `Interpreter::step` debits static opcode gas before calling the instruction. In the 43.0.2 `DUP1` handler, `stack.dup(N) == false` becomes `InstructionResult::StackOverflow`. The stack API's boolean cannot distinguish a missing source item from a full destination stack, explaining the reported empty-stack label. REVM source's `Interpreter::halt` spends all gas for OOG, but not for every other halt result; a direct-runner comparison must normalize exceptional-frame remaining gas rather than assume the raw interpreter result is already identical to EELS's `process_message` return.

### DUP1 discrepancy: meaning and assertions

The existing regression test was run and passed:

```text
cargo test -p ferreth-compiler --test rc_revm_conformance \
  revm_43_dup1_empty_stack_reports_stack_overflow_not_underflow -- --exact --nocapture
```

It confirms the repository's transaction harness reports REVM `StackOverflow` for bytecode `0x80`, while Ferreth reports guest `StackUnderflow`. Cancun EELS `dup_n` charges gas, checks `item_number >= len(stack)` and raises `StackUnderflowError`; at capacity, its subsequent `push` raises `StackOverflowError`.

The EVM semantic assertion should be **normal completion versus exceptional halt**, with comparable frame context and exceptional gas treatment. Preserve the concrete reason as per-implementation diagnostic data. A difference between `StackUnderflow` and `StackOverflow` labels alone is not evidence of a consensus-visible mismatch. Compare stack mutations only for successful execution; on an exceptional path, implementations can expose different internal intermediate stacks.

## 6. Options and recommendation

| Approach | Effort / CI / dependencies | Independence and observable fit | Assessment |
|---|---|---|---|
| A. Static, source-attributed vectors | Low; Rust-only and fast; no new dependencies | Independent if derivation/source is recorded; can assert Ferreth's stack/status | Best immediate candidate |
| B. Live EELS runner | Medium; Python plus pinned EELS environment; keep out of per-PR CI initially | Independent Cancun implementation; frame stack/gas/error exposed | Validated in the isolated Priority 1 experiment |
| C. EEST fixtures | Medium/high adapter effort; large release assets and state/transaction machinery | Official test ecosystem, but generally no internal final stack | Inspect one case; do not build a general importer now |
| C2. Historical VMTests | Low to inspect; variable adapter effort; JSON itself is lightweight | VM gas budget helps, but fork and expected-result provenance need case-by-case validation | Research-only until an actual fixture is pinned |
| D. Hybrid | Low initial cost; REVM remains routine CI, small Rust vectors; optional EELS check | Balances speed and an independent semantic anchor; grows with observable opcode support | Recommended direction to evaluate |

**Research recommendation:** keep REVM as the routine differential reference. Before adding a parser, Python runtime dependency, or EEST CI job, build a small deterministic vector set for behavior Ferreth actually observes. Derive cases from pinned Cancun semantics and cross-check with the EELS frame runner.

### Experiment follow-up

1. **EELS frame smoke test — completed.** The [Priority 1 experiment](../../research/eels-cancun/README.md) records the pinned environment and the observed stack, gas, error, and PC for successful and exceptional frames.
2. **REVM direct frame — completed.** The [Priority 2 experiment](../../research/revm-frame/README.md) records the direct interpreter results and the evidence for exceptional-frame gas settlement.
3. **DUP1 semantic classification — verified.** The [independent frame runs](../../research/revm-frame/README.md#2-empty-stack-dup1) confirm exceptional termination with different labels. Priority 3 preserves the dedicated Ferreth/REVM diagnostic regression and compares semantic termination in the general differential harness.
4. **One EEST fixture — defer until an observable output fits.** Inspect a specific Cancun fixture from `tests@v21.0.2`, including schema and exact asserted fields. The release asset is large; first use manifest/source metadata or a selected small fixture. Success means identifying an assertion Ferreth can make without full transaction/state semantics. A negative result is useful.
5. **One historical VMTest — defer until provenance is located.** Find an actual JSON case and record fixture release, fork, generator and exact schema. Compare its initial and remaining VM gas and output against Cancun rules. Do not claim final stack coverage if it is absent.
6. **Repeated GAS — after gas support exists.** For budget 10, test `GAS; GAS; STOP` expecting stack `[8, 6]` and 6 remaining; then test budgets below and at charge boundaries. Keep EVM gas separate from RV64 instruction counts.

**Incremental scale:** begin with a handful of independently sourced success, operand-order, underflow/overflow and fallthrough cases. Expand when Ferreth gains gas or new observable opcodes. Do not require EEST ingestion or an independent runner for every opcode unless later evidence justifies that cost.

## 7. Evidence, limits and references

### What was verified

- Repository code, ADRs, tests, and Cargo.lock inspected locally.
- EELS Cancun source API and operation order inspected at pinned execution-specs commit `d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3`.
- EEST archive README, current `execution-specs` release metadata and representative opcode test source inspected.
- Historical VM Tests wiki schema inspected; current `ethereum/tests` repository metadata and historical top-level paths checked. No actual current VMTest JSON was located.
- REVM 43.0.2 interpreter and stack handler source inspected; lockfile resolves revm 43.0.3 / revm-interpreter 43.0.2.
- Focused existing REVM/Ferreth DUP1 regression test executed successfully. It confirms the test's expected labels, not consensus equivalence or frame gas.
- The [EELS](../../research/eels-cancun/README.md) and [REVM](../../research/revm-frame/README.md) experiments ran Cancun frames independently. The runner READMEs contain the observed results and limits.

### Claim audit

| Claim | Classification | Evidence / limit |
|---|---|---|
| Cancun EELS exposes `process_message(Message) -> Evm`, empty-stack initialization, frame gas and returned stack/error fields | **Verified** | Pinned source inspected and the Priority 1 runner exercised the API |
| A minimal EELS frame can be constructed and run with a small dummy state | **Experimentally verified** | Priority 1 ran with a synthetic environment and empty pre-state |
| EEST code and fixture-release ownership moved to `execution-specs` | **Verified** | Archived repository README and current release metadata |
| Every historical VMTest family has migrated or has Cancun-compatible fixtures | **Incorrect / unsupported** | Current migration issue remains an open draft; no concrete fixture was verified |
| REVM 43.0.2 has a direct interpreter with explicit initial gas and inspectable result | **Experimentally verified** | Priority 2 compiled and ran the direct interpreter |
| Existing transaction gas can be normalized by subtracting 21,000 | **Incorrect as a general rule** | Depends on transaction shape and frame forwarding; current harness does not capture an equivalent frame budget |
| Empty-stack DUP1's REVM StackOverflow label is an EVM semantic mismatch | **Incorrect framing** | Test/source confirms different diagnostic; both are exceptional halts |
| A tiny set of independent source-attributed vectors is a useful near-term increment | **Proposal** | Engineering recommendation, not an accepted decision |

### Not verified or not executed

- Current per-case historical VMTests inventory, expected-result generator provenance, or Cancun annotation.
- Any released EEST fixture downloaded or executed; the inspected v21.0.2 fixture archive is approximately 939 MB.

### Primary sources

- EELS Cancun [`interpreter.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/interpreter.py), [`vm/__init__.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/__init__.py), [`state_tracker.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/state_tracker.py), [`arithmetic.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/arithmetic.py), [`stack.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/stack.py), [`control_flow.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/control_flow.py), [`gas.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/gas.py), and [`memory.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/memory.py).
- EEST [`archive notice and migration statement`](https://github.com/ethereum/execution-spec-tests/blob/main/README.md); current [`tests@v21.0.2 release`](https://github.com/ethereum/execution-specs/releases/tag/tests%40v21.0.2); representative [`frontier opcode tests`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/tests/frontier/opcodes/test_all_opcodes.py); pinned [`README`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/README.md) and [`pyproject.toml`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/pyproject.toml).
- Historical [`ethereum/tests VM Tests wiki`](https://github.com/ethereum/tests/wiki/VM-Tests); open draft [`execution-specs #2556`](https://github.com/ethereum/execution-specs/issues/2556).
- Locked REVM source: [`revm-interpreter 43.0.2 interpreter`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter.rs), [`stack instructions`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/instructions/stack.rs), [`instruction types`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter_types.rs), [`interpreter action`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter_action.rs).

## 8. Next Steps — Incremental Conformance Roadmap

The following ordered plan establishes a practical path to Ferreth's incremental EVM conformance. The sequence is intentional but may be adjusted when experimental evidence justifies it. No deadlines are imposed; the focus is on reproducible, evidence-grounded execution and minimal correctness fixes before any architectural commitment.

### Priority 1: Executable EELS Cancun research spike

**Status:** Completed — 2026-11-18

**Objective:** Establish a minimal, reproducible Python execution experiment using the official Cancun EELS implementation to validate frame-level execution semantics.

**Goals:**
- Execute a few small EVM bytecode programs against the pinned Cancun EELS revision.
- Initialize an explicit execution-frame gas budget (not transaction-level).
- Observe final stack, remaining gas, and error classification.
- Verify both successful execution and exceptional halts.
- Record exact source revision, environment, Python setup, commands, and results.

**Initial test vector:** `STOP`, `PUSH1 2 STOP`, `ADD`, `GAS`, repeated `GAS GAS STOP`, and an empty-stack `DUP1` exception.

**Scope:** Keep this experiment isolated from Ferreth's production codebase.

**Success criterion:** At least one successful and one exceptional EVM execution frame produce observable, reproducible results from the returned `Evm` object.

### Priority 2: Direct REVM frame-level execution

**Status:** Completed. See the [direct REVM experiment](../../research/revm-frame/README.md).

**Objective:** Investigate and validate a reproducible frame-equivalent execution path using Ferreth's pinned REVM 43.0.3 version, independent of transaction intrinsic gas.

**Goals:**
- Instantiate REVM's `Interpreter` directly with Cancun and an explicit frame gas budget.
- Avoid transaction intrinsic-gas (21,000) contamination.
- Inspect stack, gas remaining, and terminal result.
- Repeat the EELS smoke-test cases for direct comparison.
- Investigate whether diagnostic differences (e.g. StackUnderflow vs. StackOverflow) normalize to equivalent frame halts.

**Scope:** Compile but do not deploy; treat as a research runnable in Ferreth's test suite.

**Success criterion:** A reproducible, frame-equivalent REVM execution path suitable for future differential comparisons. This is the principal experimental prerequisite for ADR-002 frame-budget validation.

### Priority 3: Focused conformance corrections

**Status:** Implemented and verified. The locked workspace suite passed 135 tests. See RF-003 through RF-006 for regression and audit evidence.

**Objective:** Address currently identified discrepancies before introducing the gas-metering implementation.

**Goals:**
- Investigate and correct truncated PUSH1 handling to match Cancun semantics (zero-pad immediates, do not reject).
- Preserve regression coverage for existing tests.
- Document the boundary between consensus-relevant semantic failures and implementation-specific diagnostic labels.
- Retain the known empty-stack DUP1 label discrepancy (StackUnderflow vs. StackOverflow) as a regression diagnostic case without treating enum-name differences as automatic semantic mismatches.

**Scope:** Decoder and existing conformance tests, with a verification record in this document.

**Success criterion:** Supported opcode behavior agrees with Cancun semantics, and the differential harness reliably distinguishes semantic failures from diagnostic differences.

### Priority 4: ADR-002 implementation and GAS opcode

**Objective:** Implement guest-authoritative gas metering and the GAS opcode after architectural approval.

**Prerequisites:**
- Frame-level REVM and EELS experiments must be validated.
- ADR-002 must receive explicit maintainer review and approval.

**Goals:**
- Implement the gas-metering architecture defined in ADR-002.
- Preserve all ADR-001 architectural contracts.
- Implement the GAS opcode (0x5A) as part of the initial gas milestone.
- Verify exact gas boundaries for each supported opcode.
- Verify repeated GAS instructions observe correct decreasing sequence.
- Compare gas, successful execution, and exceptional results against frame-equivalent REVM and EELS references.

**Scope:** Compiler, guest ABI (gas fields), and integration tests.

**Success criterion:** The initial seven-opcode subset (STOP, PUSH1, ADD, SUB, POP, DUP1, GAS) satisfies the approved gas contract and passes the relevant Cancun conformance tests.

**Important:** Do not treat this roadmap as approval of ADR-002.

### Priority 5: Incremental opcode expansion

**Objective:** Extend Ferreth's supported opcode subset incrementally alongside compiler improvements.

**Goals:**
- Maintain REVM differential testing for fast iteration.
- Add focused Cancun-derived semantic vectors for each new opcode.
- Add exceptional and boundary tests (e.g. operand out of range, stack limits).
- Use EELS or official fixtures when they provide meaningful independent evidence.
- Measure RV64 efficiency separately from EVM gas; do not conflate instruction counts with gas costs.

**Scope:** Incremental compiler features, targeted tests, and optional fixture ingestion.

**Ongoing approach:** Avoid requiring EEST integration or Python execution in every CI run unless evidence justifies that overhead.

**Success criterion:** Conformance coverage grows alongside compiler functionality without unnecessarily increasing development complexity.

---

## Research findings 🔎

**Research date:** 2026-11-18

**Scope:** Cancun execution-frame semantics and incremental Ferreth conformance.

**Status:** Priorities 1 and 2 research and Priority 3 corrections are complete. Priority 4 architectural decisions remain subject to maintainer review.

### Experimental references

The following executable research utilities provide the supporting evidence for these findings:

- [EELS Cancun — Frame Execution Research](../../research/eels-cancun/README.md)
- [REVM Cancun — Direct Frame Execution Research](../../research/revm-frame/README.md)

The individual research READMEs contain environment configuration, source revisions, reproducible commands, experimental results, and limitations.

This section consolidates findings relevant to Ferreth's architecture, implementation, and conformance requirements.

### RF-001 — Independent frame-level execution references

**Evidence:** Experimentally verified.

Direct execution of isolated Cancun EVM frames has been successfully established using:

- EELS `process_message()`.
- REVM `Interpreter::run_plain()`.

Both runners accept explicit bytecode and initial frame gas without transaction processing or intrinsic gas accounting.

Across 18 shared experimental vectors:

- 10 successful executions produced identical final stacks and remaining gas.
- 8 exceptional executions agreed on exceptional termination.
- Differences were observed in raw gas accounting, error classification, exceptional stack contents, and terminal program counters.

EELS experiments also confirmed that message gas determines the execution budget for the tested direct execution path, independently of transaction environment gas.

**Ferreth implications:**

- Maintain frame-level execution as the comparison boundary for ADR-002.
- Preserve existing transaction-based REVM differential tests.
- Use EELS as an independent semantic reference.
- Do not conflate transaction gas, EVM frame gas, and RV64 instruction counts.

**Status:** Research complete.

### RF-002 — Exceptional halts consume remaining frame gas

**Evidence:** Experimentally verified in EELS; REVM raw execution verified experimentally and settlement behavior verified through source inspection.

Regression vector:

- Bytecode: `0x60020100`
- Instructions: `PUSH1 2; ADD; STOP`
- Initial frame gas: `20`
- EELS transaction environment gas: `100000`

| Observation | EELS Cancun | Direct REVM Cancun |
|---|---:|---:|
| Raw remaining gas | 0 | 14 |
| Raw consumed gas | 20 | 6 |
| Terminal stack | `[]` | `[2]` |
| Terminal PC | 2 | 3 |
| Error | StackUnderflowError | StackUnderflow |

The difference arises from distinct execution boundaries.

EELS performs exceptional-halt gas settlement within `process_message()`.

REVM's direct interpreter charges static opcode gas but may preserve residual gas for exceptional results other than OutOfGas.

Source inspection of REVM confirmed:

- `Interpreter::halt()` explicitly consumes remaining gas for OutOfGas.
- `handle_reservoir_remaining_gas()` consumes the remaining regular gas of exceptionally halted child frames.
- `last_frame_result()` uses the same settlement function for the initial frame.

Verified source versions:

- `revm`: 43.0.3
- `revm-interpreter`: 43.0.2
- `revm-handler`: 43.0.3

The settled REVM remaining gas of zero is source-derived, not directly observed through execution of the complete handler pipeline.

**Ferreth requirements:**

- Exceptional frame termination must consume all remaining frame gas.
- Successful execution must preserve unused frame gas.
- REVERT must remain distinct from exceptional termination.
- Raw REVM interpreter gas must be normalized when comparing exceptional frame outcomes.
- Later-fork state-gas and reservoir accounting must not be introduced into Cancun semantics.

Normalization must apply specifically to exceptional halts, not to every unsuccessful result.

**Required regression test:**

`stack_underflow_consumes_remaining_frame_gas`

Assertions:

- Bytecode: `0x60020100`.
- Initial frame gas: `20`.
- Exceptional frame termination.
- Remaining frame gas: `0`.
- Guest diagnostic: `StackUnderflow`.

Do not require the exceptional terminal stack to reproduce reference-interpreter mutations.

**Architectural decision pending:** Define which Ferreth execution boundary owns exceptional-halt gas settlement during ADR-002 review.

**Status:** Research complete; implementation pending ADR-002.

### RF-003 — Error precedence and classification are implementation-specific diagnostics

**Evidence:** Experimentally verified in EELS and REVM.

EELS and REVM may validate gas availability and stack operands in different orders.

Empty-stack ADD (`0x01`):

| Initial frame gas | EELS | REVM |
|---:|---|---|
| 0 | StackUnderflowError | OutOfGas |
| 2 | StackUnderflowError | OutOfGas |
| 3 | StackUnderflowError | StackUnderflow |

Empty-stack DUP1 (`0x80`) with initial frame gas `3`:

| Implementation | Diagnostic |
|---|---|
| EELS | StackUnderflowError |
| REVM | StackOverflow |

All cases terminate exceptionally.

EELS ADD extracts operands before charging its ordinary opcode gas. REVM charges static gas before dispatching the opcode handler.

The observed diagnostic differences do not independently establish consensus-semantic disagreement.

**Ferreth requirements:**

- Preserve meaningful guest error classifications.
- Define a deterministic error-precedence policy.
- Distinguish semantic termination from diagnostic enum equality.
- Retain the existing empty-stack DUP1 discrepancy as regression coverage.

**Architectural decision pending:** Review ADR-002's proposed ordering of gas checks and stack validation.

**Priority 3 status:** Implemented and verified. The [differential helper](../../crates/ferreth-compiler/tests/rc_revm_conformance.rs#L98) requires REVM success or halt as expected and checks Ferreth's exact guest status and exception code independently. Host errors and unexpected termination fail the comparison. The dedicated [empty-stack DUP1 regression](../../crates/ferreth-compiler/tests/rc_revm_conformance.rs#L284) still requires REVM `StackOverflow` and Ferreth `StackUnderflow`. The seeded suite ran 1,000 successful and 100 exceptional cases, including DUP1 underflows through the shared helper. Gas-related error precedence remains pending under Priority 4.

### RF-004 — Exceptional terminal stacks are not portable conformance results

**Evidence:** Experimentally verified.

| Bytecode | Initial gas | EELS stack | REVM stack |
|---|---:|---|---|
| `60020100` | 20 | `[]` | `[2]` |
| `600260030100` | 8 | `[]` | `[2, 3]` |

EELS may mutate the stack before detecting an exceptional condition.

REVM preserves operands in these observed cases.

These differences reflect internal execution ordering rather than successful EVM computation results.

**Ferreth requirements:**

- Compare final stack contents for successful execution.
- Preserve exceptional terminal stack observations for diagnostics.
- Do not require exceptional terminal stacks to match independent reference implementations.

Ferreth may preserve stronger internal stack invariants without introducing a consensus-semantic discrepancy.

**Priority 3 audit:** No correction required. The differential helper compares stacks after successful execution and reports observed exceptional stacks in failure diagnostics. The passing workspace suite continues to verify Ferreth's own stack and `s1` preservation on guest errors. See the [ADD and SUB guest regressions](../../crates/ferreth-compiler/tests/rv64_push1_program.rs#L212) and [DUP1 guest regressions](../../crates/ferreth-compiler/tests/rv64_dup1.rs#L44).

### RF-005 — Truncated PUSH immediates are zero-padded

**Evidence:** Experimentally verified in EELS and REVM.

Regression vector:

- Bytecode: `0x60`
- Instruction: PUSH1 with a missing immediate.
- Initial frame gas: `3`.

Both implementations produce:

- Successful completion.
- Final stack: `[0]`.
- Remaining gas: `0`.

Before Priority 3, Ferreth rejected the vector before guest execution. The new guest regression reproduced the discrepancy with Cargo exit 101 and the following error:

```text
Compile(Decode(TruncatedPush { offset: 0, opcode: 96, expected: 1, available: 0 }))
```

**Ferreth requirements:**

- Correct truncated PUSH1 decoding to match Cancun zero-padding semantics.
- Preserve the distinction between original bytecode length and logical immediate width.
- Add focused regression coverage.
- Apply the same zero-padding rule to all decoded PUSH widths. The RV64 compiler lowers PUSH1 and still reports `UnsupportedInstruction` for wider widths.

**Priority 3 status:** Implemented and verified. The same guest regression passed after the fix with normal completion, exception code zero, and stack `[0]`. The [decoder regression](../../crates/ferreth-compiler/src/lib.rs#L343) covers `60`, `61ab`, `61`, `61abcd`, and a truncated PUSH32 at a nonzero offset. The [guest regression](../../crates/ferreth-compiler/tests/rv64_push1_program.rs#L121) and [directed Cancun differential suite](../../crates/ferreth-compiler/tests/rc_revm_conformance.rs#L177) cover `0x60` through RV64 execution. A [compiler boundary test](../../crates/ferreth-compiler/tests/rv64_push1_program.rs#L312) verifies rejection of both complete and truncated PUSH2 at the original byte offset. Repeat runs of the [EELS](../../research/eels-cancun/README.md#4-truncated-push1) and [REVM](../../research/revm-frame/README.md#3-truncated-push1) frame vectors with gas 3 both produced stack `[0]` and zero remaining gas.

```bash
cargo test --locked -p ferreth-compiler --test rv64_push1_program truncated_push1_zero_pads_missing_immediate -- --exact
```

### RF-006 — Terminal program counters may differ across execution engines

**Evidence:** Experimentally verified.

REVM advances its instruction pointer before charging static gas and dispatching the instruction handler.

Consequently, exceptional terminal PC values may differ from EELS.

Representative observations:

| Bytecode | Initial gas | EELS PC | REVM PC |
|---|---:|---:|---:|
| `01` | 0 | 0 | 1 |
| `60020100` | 20 | 2 | 3 |
| `600260030100` | 8 | 4 | 5 |
| `5a00` | 1 | 0 | 1 |
| `60` | 3 | 2 | 3 |

The truncated PUSH1 case also demonstrates that terminal PC differences can occur after successful execution.

REVM's analyzed bytecode padding and implicit STOP execution are a possible explanation for the truncated PUSH1 PC difference. This explanation remains unverified.

**Ferreth requirements:**

- Retain terminal PC observations for diagnostics.
- Do not require identical terminal PC values across execution engines.
- Distinguish terminal interpreter PC from the consensus-relevant behavior of executing the PC opcode.

This finding does not relax the future requirement to implement the PC opcode correctly.

**Priority 3 audit:** No correction required. Source inspection found no cross-engine terminal EVM PC equality assertion. The [RV64 executor tests](../../crates/ferreth-compiler/tests/rv64_executor.rs#L34) validate guest exit boundaries under ADR-001. The truncated PUSH1 PC mechanism in REVM remains unverified.

### RF-007 — GAS charges before observing remaining frame gas

**Evidence:** Experimentally verified in EELS and REVM.

Both implementations produced identical successful stack and gas results:

| Bytecode | Initial gas | Final stack | Remaining gas |
|---|---:|---|---:|
| `5a00` | 10 | `[8]` | 8 |
| `5a5a00` | 10 | `[8, 6]` | 6 |
| `5a00` | 2 | `[0]` | 0 |

Both implementations terminate exceptionally with zero remaining gas when executing GAS with an initial budget of zero or one.

**Ferreth requirements:**

- GAS must cost exactly two gas units under Cancun.
- GAS must charge its cost before observing remaining frame gas.
- The value pushed onto the EVM stack must reflect post-charge gas.
- Repeated GAS instructions must observe the decreasing frame budget.
- Execution with exactly two available gas units must succeed.
- Execution with fewer than two available gas units must terminate exceptionally.

Proposed regression tests:

- `gas_pushes_remaining_gas_after_charge`
- `repeated_gas_observes_decreasing_budget`
- `gas_succeeds_at_exact_cost_boundary`
- `gas_fails_below_cost_boundary`

**Status:** Pending ADR-002 implementation and GAS opcode support.

### Consolidated conformance policy

The experimental evidence supports the following comparison policy for the currently investigated Cancun execution model.

| Observation | Successful execution | Exceptional halt |
|---|---|---|
| Termination classification | Required | Required |
| Settled remaining frame gas | Required | Required |
| Final stack contents | Required | Diagnostic |
| Exact error classification | Not applicable | Diagnostic |
| Terminal program counter | Diagnostic | Diagnostic |
| Raw interpreter gas | Useful | Diagnostic |

REVERT must be treated as a distinct termination category that preserves unused frame gas. Its behavior was not experimentally validated in these research spikes.

Implementation-specific diagnostics may still be asserted in Ferreth's own regression tests when they represent an explicitly defined guest contract.

### Implementation traceability

| Finding | Target | Status |
|---|---|---|
| RF-001 | Frame-level differential testing | Research complete |
| RF-002 | ADR-002 and exceptional-halt regression | Pending Priority 4 |
| RF-003 | Shared differential helper and dedicated empty-stack DUP1 regression | Harness verified in Priority 3. Gas-related precedence pending Priority 4 |
| RF-004 | Successful stack comparison and guest stack preservation regressions | Audited in Priority 3. No correction required |
| RF-005 | Decoder, RV64 guest, compiler boundary, and Cancun differential regressions | Implemented and verified in Priority 3 |
| RF-006 | Terminal PC diagnostics and RV64 exit-boundary tests | Audited in Priority 3. No correction required |
| RF-007 | GAS implementation and boundary tests | Pending Priority 4 |

### Research completion

Priorities 1 and 2 have successfully established independent, executable Cancun reference paths.

Priority 3 completed the focused corrections. The workspace suite, differential suite, formatting, compilation, and strict lint checks passed.

Outstanding research limitations remain documented in the individual runner READMEs.

These findings constitute research evidence and proposed implementation requirements. They do not modify or approve ADR-002.

---

## Final assessment

The executable EELS and REVM frame references now support the verified Priority 3 corrections. The Ferreth differential suite uses transaction-driven execution to check termination and successful stacks. Equivalent-budget frame-gas comparisons, exceptional gas settlement, and gas-related error precedence remain part of ADR-002 review and Priority 4. Historical VMTests remain unverified at the concrete-fixture level.

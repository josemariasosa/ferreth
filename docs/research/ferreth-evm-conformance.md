# Independent EVM Conformance Research for Ferreth

**Research snapshot:** 17 November 2026

**Fork under test:** Cancun

**Status:** Non-normative research; no architecture or implementation decision

**Scope:** Ferreth's six implemented opcodes (`STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, `DUP1`) and possible `GAS` support.

## Executive summary

The useful near-term outcome is not a new fixture framework. Keep the existing REVM differential tests for fast iteration, and consider a small set of Cancun vectors with explicit provenance. EELS has a real Cancun frame entry point that exposes stack, gas and error, but its context construction and execution have not been run here. A direct REVM interpreter API is source-verified, but no frame runner has been compiled or executed.

The original research contained several important overstatements:

1. **EEST did move.** The `ethereum/execution-spec-tests` repository was archived read-only on 2 July 2026. Its README says code was migrated to `ethereum/execution-specs` and fixture releases are now published there. This is a repository and release move, not evidence that every old fixture family has already been migrated.
2. **The EELS `process_message` API is real, but not a ready-made bytecode runner.** At the current `execution-specs` revision it is `process_message(message: Message) -> Evm`, in the Cancun interpreter module. It seeds an empty stack and the message's frame gas, then returns the internal EVM object. Constructing the required environment and transaction state remains necessary; no end-to-end Python experiment succeeded here.
3. **Stack-operation ordering differs between EELS and REVM.** Cancun EELS often pops operands before charging (e.g. ADD, SUB, POP), while REVM's interpreter charges the static opcode gas before dispatching its handler. Both treat exceptional halt as a failed frame; internal error labels and partial stack observations can differ. These differences are not, by themselves, externally observable consensus disagreements.
4. **The known empty-stack DUP1 label difference reproduces.** Ferreth's existing REVM 43 integration test reports REVM `StackOverflow` and guest `StackUnderflow`. Cancun EELS checks the missing DUP operand and raises `StackUnderflowError`. REVM's DUP handler maps a failed stack duplication to `StackOverflow`. This is a diagnostic taxonomy mismatch; do not make enum-name equality the conformance criterion.
5. **Current Ferreth behavior matters.** The compiler supports the six opcodes, but not GAS; the decoder rejects truncated PUSH1 immediates while Cancun EELS zero-pads a truncated immediate. Existing REVM tests execute through a transaction with a 1,000,000 gas limit and inspect the stack; they are not frame-gas comparisons.
6. **Historical VMTests need a provenance caveat.** The `ethereum/tests` repository is not archived, but the old VM Tests wiki is from 2015, and the current `develop` branch does not expose the historical top-level `VMTests`/`VMTest` paths. The wiki describes execution gas and remaining gas, but not an initial or expected final stack. No specific current JSON fixture was verified.

These are research findings, not changes to [ADR-001](../adr/001-evm-guest-state-abi.md) or [ADR-002](../adr/002-cancun-evm-gas-metering.md). ADR-002 remains **Proposed** and its frame-budget / error-precedence decisions need review against the source-order evidence below.

## 1. Repository-grounded Ferreth context

Repository observations:

- [compiler.rs](../../crates/ferreth-compiler/src/compiler.rs) compiles `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, and `DUP1`. It stops decoding at `STOP`. Reaching the end of bytecode completes normally. The decoder recognizes PUSH1–PUSH32, but the compiler rejects all PUSH widths other than PUSH1.
- [lib.rs](../../crates/ferreth-compiler/src/lib.rs) rejects truncated PUSH immediates with `DecodeError::TruncatedPush`. That differs from Cancun EELS `buffer_read`, which right-pads the requested bytes with zero. This is an implementation discrepancy to track; it is not silently resolved by this research.
- [dup.rs](../../crates/ferreth-compiler/src/rv64/dup.rs) checks underflow before capacity and leaves the stack unchanged on either error. [guest.rs](../../crates/ferreth-compiler/src/rv64/guest.rs) defines the 1024-word stack and its guest-side stack validation.
- [rc_revm_conformance.rs](../../crates/ferreth-compiler/tests/rc_revm_conformance.rs) runs REVM with `SpecId::CANCUN`, through `inspect_tx`, with transaction gas limit 1,000,000. Its inspector captures the interpreter stack after instructions. It contains an explicit test for the empty-stack DUP1 diagnostic mismatch.
- [execution.rs](../../crates/ferreth-compiler/src/execution.rs) also uses Cancun REVM, but its transaction path returns call output rather than frame stack/gas.
- [Cargo.lock](../../Cargo.lock) resolves `revm` 43.0.3 and `revm-interpreter` 43.0.2. The compiler crate depends directly on `revm` 43.0.3; it does not declare a separate direct `revm-interpreter` dependency.
- [benchmark.rs](../../crates/ferreth-compiler/src/benchmark.rs) measures RV64 instruction counts, not EVM gas.

The current guest ABI has status, provisional exception code and final stack pointer; gas is not part of the ABI. GAS and gas metering are not implemented. ADR-001 is Accepted; ADR-002 is Proposed. This report does not authorize implementation or change either ADR.

The truncated-PUSH mismatch is material for bytecode semantics, but it does not mean every legacy fixture is appropriate. The outstanding gas-order observations also challenge ADR-002's stated assumption that all opcode gas checks precede stack handling; this is evidence for review, not an instruction to edit the frozen ADR.

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
- It snapshots transaction state even for simple arithmetic, so supplying only code and gas is insufficient. The Python `BlockState`, `TransactionState`, `BlockEnvironment`, `TransactionEnvironment` and `Message` dataclasses can be constructed directly, but a valid pre-state/context still needs to be supplied. The exact minimal setup was not executed.
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

For a successful frame, `GAS; GAS; STOP` with frame budget 10 should push 8 then 6 and finish with 6 gas. This is source-derived, not an observed experiment. `GAS` with less than 2 gas is OOG before a value is pushed. The Cancun `stack.push` limit is 1024; `DUP1` at capacity fails on the push after its gas charge.

### Installation and experiment status

The pinned repository `pyproject.toml` identifies the package as `ethereum-execution`, requires Python >=3.11, and lists its runtime dependencies. Its README says Python 3.11–3.14 are supported (3.12 recommended) and uses `uv sync`; the `ethereum-spec-evm` CLI is supplied by the `ethereum-execution-testing` workspace package and runs transitions/state tests, not arbitrary frames. EEST tooling is in the repository's test dependency group, not a lightweight, preconfigured Ferreth runner. A reproducible run should pin the repository SHA above, record Python and lockfile, and use the repository's supported environment tooling.

**Experiment attempted:** `python3 -c 'import ethereum; print(ethereum.__file__)'`
**Observed:** `ModuleNotFoundError: No module named 'ethereum'`. No dependencies were installed and no Ferreth files or manifests were changed. Therefore no EELS outputs are claimed. The exact state/environment construction remains unverified at runtime.

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

Ferreth's conformance test builds a Cancun REVM context and calls `inspect_tx` on a transaction with a 1,000,000 gas limit. Its `StackInspector` saves stack contents at interpreter initialization and after successful steps. This is useful whole-program differential testing, but it does not supply Ferreth with a matching EVM frame budget or compare final frame gas. Subtracting 21,000 from the transaction gas limit/receipt is not generally a frame-equivalence proof: transaction calldata, access lists, creation, execution setup, refunds and fork-specific accounting can affect transaction-level figures.

A carefully constrained transaction might be normalized if its envelope, intrinsic costs, call forwarding and pre-op gas are all controlled and verified. That is not what the current harness does, and no such normalization was established here. Transaction results do not publish the final EVM stack; the current inspector obtains stack observations separately.

### Low-level API found in pinned source

The `revm-interpreter` 43.0.2 source resolved by Ferreth's lockfile exposes a low-level interpreter. `Interpreter::new` takes `SharedMemory`, `ExtBytecode`, `InputsImpl`, static flag, `SpecId` and explicit `gas_limit: u64`. `Interpreter` exposes its stack and gas fields; the stack can be seeded using its stack API. `run_plain(instruction_table, gas_table, host)` returns an `InterpreterAction`, whose return variant contains an `InterpreterResult` with `InstructionResult` and gas.

This establishes that direct interpreter execution is possible in principle without transaction intrinsic gas. It is not yet a validated Ferreth runner: it still requires correct `ExtBytecode`, input, gas/instruction tables and a `Host`; construction and result extraction have not been compiled or exercised in a scratch program. The exact source is [`interpreter.rs`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter.rs), [`interpreter_types.rs`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter_types.rs) and [`instructions/stack.rs`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/instructions/stack.rs).

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
| B. Live EELS runner | Medium; Python plus pinned EELS environment; keep out of per-PR CI initially | Independent Cancun implementation; frame stack/gas/error exposed | Worth one isolated feasibility experiment; context setup is unproven |
| C. EEST fixtures | Medium/high adapter effort; large release assets and state/transaction machinery | Official test ecosystem, but generally no internal final stack | Inspect one case; do not build a general importer now |
| C2. Historical VMTests | Low to inspect; variable adapter effort; JSON itself is lightweight | VM gas budget helps, but fork and expected-result provenance need case-by-case validation | Research-only until an actual fixture is pinned |
| D. Hybrid | Low initial cost; REVM remains routine CI, small Rust vectors; optional EELS check | Balances speed and an independent semantic anchor; grows with observable opcode support | Recommended direction to evaluate |

**Research recommendation:** keep REVM as the routine differential reference. Before adding a parser, Python runtime dependency, or EEST CI job, build a small deterministic vector set for behavior Ferreth actually observes. Independently derive cases from pinned Cancun semantics and cross-check with EELS once its frame setup is executable. A candidate is not independent merely because its result matches REVM.

### Candidate experiments

1. **EELS frame smoke test — worth doing next.** Pin the commit above and Python environment; construct Cancun block/transaction/message state with an empty initial stack and run: `STOP` (`0x00`, gas 0); `PUSH1 2; STOP` (`0x600200`, gas 3); `SUB` (`0x600260030300`, gas 20); `ADD` (`0x01`, budgets 0, 2 and 3); `GAS; STOP` (`0x5a00`, gas 10); repeated GAS (`0x5a5a00`, gas 10); and empty-stack `DUP1` (`0x80`, gas 3). Capture exact setup and `stack`, `gas_left`, `error`, `pc`. Expected successful observations are `[2]`/0 gas for PUSH1, `[1]`/11 gas for SUB and `[8]`/8 gas for GAS; exceptional cases should return a specific `error` and zero terminal gas. Do not assert exceptional stack state across implementations. Success means deterministic successful and exceptional results from returned `Evm`; failure must record the setup blocker. No EELS run has yet occurred.
2. **REVM direct frame — worth doing after the EELS smoke test.** Using locked REVM 43.0.3 / interpreter 43.0.2, instantiate `Interpreter` with Cancun, explicit budget and bytecode; seed a stack for a successful arithmetic vector; run to `InterpreterAction::Return`. Test `STOP` with 0 gas; empty-stack ADD at 0, 2 and 3; `GAS; GAS; STOP` (`0x5a5a00`) at 10; `PUSH1 2; PUSH1 3; ADD; STOP` (`0x600260030100`) at 8 and 9; empty-stack DUP1 (`0x80`) with sufficient gas. Record `InstructionResult`, raw interpreter gas and stack. For exceptional cases, compare semantic halt and normalized frame gas, not raw intermediate stack. Success means the exact call path compiles and returns these values without transaction execution. Not executed here.
3. **DUP1 semantic classification — partly resolved.** Current Ferreth/REVM transaction test is executed; Cancun source shows EELS underflow versus REVM overflow diagnostic. A direct EELS/REVM frame run should still confirm whether raw intermediate gas/stack differences normalize to the same external exceptional result. Do not file an upstream correctness issue based only on the name.
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

### Claim audit

| Claim | Classification | Evidence / limit |
|---|---|---|
| Cancun EELS exposes `process_message(Message) -> Evm`, empty-stack initialization, frame gas and returned stack/error fields | **Verified** | Pinned Cancun source inspected; not executed here |
| A minimal EELS frame can be constructed and run with a small dummy state | **Implementation-dependent** | Required structures are known; no runnable setup was produced |
| EEST code and fixture-release ownership moved to `execution-specs` | **Verified** | Archived repository README and current release metadata |
| Every historical VMTest family has migrated or has Cancun-compatible fixtures | **Incorrect / unsupported** | Current migration issue remains an open draft; no concrete fixture was verified |
| REVM 43.0.2 has a direct interpreter with explicit initial gas and inspectable result | **Verified at source level** | Constructor and `run_plain` inspected; no frame runner compiled |
| Existing transaction gas can be normalized by subtracting 21,000 | **Incorrect as a general rule** | Depends on transaction shape and frame forwarding; current harness does not capture an equivalent frame budget |
| Empty-stack DUP1's REVM StackOverflow label is an EVM semantic mismatch | **Incorrect framing** | Test/source confirms different diagnostic; both are exceptional halts |
| A tiny set of independent source-attributed vectors is a useful near-term increment | **Proposal** | Engineering recommendation, not an accepted decision |

### Not verified or not executed

- EELS Python frame execution; active Python environment lacks the `ethereum` module.
- REVM direct-frame invocation and gas/stack extraction in an executable Ferreth runner.
- Current per-case historical VMTests inventory, expected-result generator provenance, or Cancun annotation.
- Any released EEST fixture downloaded or executed; the inspected v21.0.2 fixture archive is approximately 939 MB.
- EELS/REVM `GAS` numeric outputs in a running experiment. Numerical expectations above are derived from Cancun source, not measurements.

### Primary sources

- EELS Cancun [`interpreter.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/interpreter.py), [`vm/__init__.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/__init__.py), [`state_tracker.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/state_tracker.py), [`arithmetic.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/arithmetic.py), [`stack.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/stack.py), [`control_flow.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/control_flow.py), [`gas.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/gas.py), and [`memory.py`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/memory.py).
- EEST [`archive notice and migration statement`](https://github.com/ethereum/execution-spec-tests/blob/main/README.md); current [`tests@v21.0.2 release`](https://github.com/ethereum/execution-specs/releases/tag/tests%40v21.0.2); representative [`frontier opcode tests`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/tests/frontier/opcodes/test_all_opcodes.py); pinned [`README`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/README.md) and [`pyproject.toml`](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/pyproject.toml).
- Historical [`ethereum/tests VM Tests wiki`](https://github.com/ethereum/tests/wiki/VM-Tests); open draft [`execution-specs #2556`](https://github.com/ethereum/execution-specs/issues/2556).
- Locked REVM source: [`revm-interpreter 43.0.2 interpreter`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter.rs), [`stack instructions`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/instructions/stack.rs), [`instruction types`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter_types.rs), [`interpreter action`](https://docs.rs/crate/revm-interpreter/43.0.2/source/src/interpreter_action.rs).

## 8. Next Steps — Incremental Conformance Roadmap

The following ordered plan establishes a practical path to Ferreth's incremental EVM conformance. The sequence is intentional but may be adjusted when experimental evidence justifies it. No deadlines are imposed; the focus is on reproducible, evidence-grounded execution and minimal correctness fixes before any architectural commitment.

### Priority 1: Executable EELS Cancun research spike

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

**Objective:** Address currently identified discrepancies before introducing the gas-metering implementation.

**Goals:**
- Investigate and correct truncated PUSH1 handling to match Cancun semantics (zero-pad immediates, do not reject).
- Preserve regression coverage for existing tests.
- Document the boundary between consensus-relevant semantic failures and implementation-specific diagnostic labels.
- Retain the known empty-stack DUP1 label discrepancy (StackUnderflow vs. StackOverflow) as a regression diagnostic case without treating enum-name differences as automatic semantic mismatches.

**Scope:** Compiler, decoder, and test suite only.

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

## Final assessment

The prior research should not be used unchanged to plan implementation. The EELS frame path and EEST migration claims are now grounded in primary sources; the REVM API is source-verified but not executed; historical VMTests remain unverified at the concrete-fixture level. Ferreth's existing REVM differential suite remains useful, but it compares transaction-driven execution and internal diagnostics, not the proposed frame-gas contract.

The most useful next independent evidence is a successful EELS frame smoke test, followed by a minimal direct REVM interpreter experiment. Until then, prefer narrowly scoped source-attributed vectors, keep source/provenance distinct from implementation observations, and treat this report as non-normative input to future review—not an ADR decision.

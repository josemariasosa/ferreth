# ADR-001 Initial Implementation Audit

- **Date:** 2026-11-17
- **Scope:** ADR-001 implementation review only; no Rust, manifest, test,
  benchmark, runbook, or historical-artifact changes were made.
- **Classification:** Mostly compliant with gaps

## Executive summary

The initial ADR-001 scope is implemented and exercised for `STOP`, `PUSH1`,
`ADD`, and `SUB`. The accepted memory map, ascending 1024-word guest stack,
little-endian four-limb U256 representation, `s0`/`s1` convention, result
record, ordered terminal publication, PC sentinel, instruction limit, and
rvemu adapter are present. RO isolated operations and RC whole-program
execution use generated RV64 code and share the guest operation generators;
the demonstrated complete program `600560030100` finishes with `[8]` after 70
RV64 instructions.

The implementation is not a complete EVM, which ADR-001 explicitly excludes.
The main validation gap is that the current differential suite proves the
pointer-core arithmetic against REVM and tests compiled guest programs with
fixed expectations, but does not directly compare RC whole-program guest
results with REVM over a meaningful program corpus. The checked-in arithmetic
baseline named by the benchmark runbook is also absent, so the documented
artifact comparison cannot be performed even though the command reproduces
the stated measurements.

## Findings

| ID | Severity | Component | Finding | Evidence | Recommendation |
|---|---|---|---|---|---|
| F-001 | Medium | RC differential validation | No test directly differentially compares `execute_guest_evm_bytecode` / compiled guest state to REVM across supported multi-opcode programs. Existing random-program parity uses `execution::ProgramRunner`, whose stack transitions are Rust-managed. | `tests/rv64_execution.rs` (`mixed_arithmetic_random_programs`, `ProgramRunner`); RC path is `orchestrator.rs::execute_guest_evm_bytecode`. | Add deterministic directed and seeded randomized RC-vs-REVM tests that compare terminal status and full live stack for the supported subset. |
| F-002 | Low | Benchmark evidence | `docs/runbooks/benchmark.md` links to `crates/ferreth-compiler/runs/rv64-arithmetic.json`, but no `runs/` artifact exists in the checked-out tree. The documented comparison script therefore cannot run as written. | `rg --files crates/ferreth-compiler` found no report JSON; benchmark runbook “Checked-in historical baseline” section. | Restore the approved historical artifact or amend the runbook in a separately authorized documentation change; do not synthesize or overwrite it during a benchmark run. |
| F-003 | Low | Lint hygiene | Required clippy verification exits successfully but emits nine warnings: eight `manual_is_multiple_of` warnings and one `single_element_loop` warning. | `cargo clippy --workspace --all-targets`, 2026-11-17. | Resolve the warnings in a separately authorized cleanup; no correctness issue was observed. |
| F-004 | Informational | EVM coverage | Decoder support is deliberately broader than RC: it parses `PUSH1`–`PUSH32`, while RC compiles only `PUSH1`, `ADD`, `SUB`, and `STOP`. This is an explicit rejected-at-compile behavior, not silent substitution. | `src/lib.rs::decode_detailed`; `src/compiler.rs::compile_evm_bytecode`; `tests/rv64_push1_program.rs::bytecode_rejects_unimplemented_or_malformed_input_before_execution`. | Retain the explicit error behavior; add push-width compilation only in a future opcode milestone. |
| F-005 | Informational | Executor scope | rvemu 0.0.11 is the sole executor. The native stack reservation is not initialized because generated code does not use `sp`; no native RV64 execution or alternative backend exists. | `src/rv64/executor/rvemu.rs`; `src/rv64/guest.rs::initialize_guest_state`; ADR-001 §11–12. | No ADR-001 initial-scope fix required. Evaluate another backend only after compatibility and counting-semantics research. |

F-001 is a missing-validation finding, not a confirmed compiler defect.
F-002 and F-003 are documentation/hygiene findings. F-004 and F-005 are
documented scope boundaries, not defects.

## Requirement traceability

| ADR-001 requirement | Implementation evidence | Test / execution evidence | Status |
|---|---|---|---|
| Fixed half-open map, aligned code, exit slot, stack capacity | `src/rv64/memory.rs` | unit layout tests; `tests/rv64_executor.rs` | Implemented |
| Four little-endian limbs, ascending stack, `s0`/`s1`, separate native `sp` | `src/rv64/guest.rs`, `src/rv64/memory.rs` | `rv64_guest_memory`, `rv64_guest_initialization` | Implemented |
| RUNNING initialization and 24-byte status/exception/final-`s1` record | `guest.rs::initialize_guest_state`, `GuestMetadata` | `rv64_guest_initialization`, `rv64_guest_result` | Implemented |
| Final pointer then exception then status; expected exit boundary before result acceptance | termination publishers in `compiler.rs`; `RvemuExecutor::execute_until`; `read_guest_result` | `rv64_guest_result`, `rv64_executor` | Implemented |
| Normal STOP and stack error exit remain distinct from executor faults/limits | `compiler.rs`, `executor/mod.rs` | `rv64_push1_program`, `rv64_executor`; CLI `600101` gave `GuestException` | Implemented |
| PUSH1 bounds; ADD/SUB underflow, aliasing, modular arithmetic and operand order | `rv64/push.rs`, `rv64/add.rs`, `rv64/sub.rs` | guest push/add/sub tests and REVM arithmetic tests | Implemented |
| One RV64 executable without Rust per-opcode callbacks | `compiler.rs::compile_evm_bytecode`; `orchestrator.rs::execute_linked_program` | `rv64_push1_program`; CLI `600560030100` | Implemented |
| RO/RC share guest ABI and arithmetic opcode generators | RO calls `generate_guest_add/sub`; RC calls same generators | mixed-linker preservation tests | Implemented |
| Backend-neutral executor boundary | `rv64/executor/mod.rs::Rv64Executor`; rvemu adapter isolated in `rvemu.rs` | `rv64_executor` | Implemented |
| REVM reference is explicit and isolated | `execution.rs::run_evm_bytecode` selects `SpecId::CANCUN`; `Cargo.toml` uses REVM 43.0.3 | `rv64_execution.rs::revm_reference_uses_cancun_and_reports_execution_errors`; arithmetic benchmark | Partially implemented (whole-program RC differential gap F-001) |
| `opcode_core`, `opcode_guest`, `program_guest` dynamic instruction metrics | `execution.rs`, `orchestrator.rs`, `benchmark.rs` | both benchmark commands below | Implemented |
| Native executor / alternative backend | No implementation required by ADR-001 initial scope | N/A | Not in initial scope |

## Test inventory

`cargo test --workspace` executed 109 tests: 37 library unit tests and 72
integration tests. Categories are:

| Category | Coverage |
|---|---|
| Unit | Decoder, U256 helpers, map/access validation, metadata encoding/validation, stack preconditions, deterministic benchmark JSON, and isolated orchestration helpers. |
| Guest ABI integration | Initialization/reinitialization, word encoding, terminal record validation, `s0`/`s1` checks, exit sentinel, fault/limit distinction. |
| Compiler / RC integration | PUSH1 linking, explicit STOP and unreachable trailing bytes, ADD/SUB linking, underflow/overflow publication, one-run program execution. |
| Executor integration | DRAM bounds, register/memory round trip, sentinel semantics, instruction limits, execution faults. |
| Differential | Directed and randomized ADD/SUB pointer-core comparisons against REVM; REVM `CANCUN` configuration test. |
| Negative paths | Unsupported/truncated bytecode, underflow/overflow, invalid metadata, changed live registers, access bounds, malformed code. |

No authoritative historical total is recorded in the inspected repository, so
there is no basis to claim that a prior count changed or that coverage was
lost. The meaningful untested behavior is F-001: full compiled guest programs
against REVM, including full live-stack comparison and terminal exceptions.

## Test and benchmark results

All commands were executed from the workspace root on 2026-11-17.

| Command | Actual outcome |
|---|---|
| `cargo fmt --all --check` | Passed (exit 0). |
| `cargo test --workspace` | Passed: 109 passed, 0 failed. |
| `cargo clippy --workspace --all-targets` | Passed (exit 0) with 9 warnings; see F-003. |
| `cargo run -p ferreth-compiler -- execute --bytecode 600560030100` | Passed: final status `Normal`, final stack `[8]`, dynamic RV64 count `70`. |
| `cargo run -p ferreth-compiler -- bench --op ALL --cases 1000 --seed 42 --format json` | Passed: ADD and SUB each 1,012/1,012, zero failures, 34 instructions/case, expected hashes and totals. |
| `cargo run -p ferreth-compiler -- bench guest --format json` | Passed: all runbook reference values match exactly, including 17/49 opcode totals and 70 for `600560030100`. |
| `translate --op ADD`; `execute --op PUSH1 --value 5` | Passed (documented CLI examples). |
| `execute --bytecode 600101` | Passed expected exceptional path: `GuestException`, stack `[1]`, 21 instructions. |
| `git diff --check` | Run after documentation edits; passed (exit 0). |

The arithmetic baseline output was ADD hash `fnv1a64:0xf1fff32ff47962f3` and
SUB hash `fnv1a64:0x9f008f320eeb7a7b`; each had 34,408 total dynamically
executed instructions. Guest totals were PUSH1 17, isolated ADD/SUB 49, and
the explicit-STOP complete program 70. The executor reports successful RV64
instructions after execution and observes the exit sentinel before fetch, so
the sentinel contributes zero instructions. Body and termination counts are
separately observed by two executor runs in `orchestrator.rs`; benchmark
generation is host-side and does not alter guest counts. The deterministic
JSON tests and repeated guest report test pass. Schema fields inspected match
the benchmark runbook; only the referenced historical artifact is missing.

## Remaining work

1. **Required fixes for ADR-001 compliance:** None confirmed for the initial
   implemented opcode scope.
2. **Test and validation improvements:** Add direct RC-to-REVM program
   differential tests (F-001); restore or reconcile the missing historical
   benchmark evidence artifact (F-002); clean the clippy warnings (F-003).
3. **Future compiler features:** Compile additional PUSH widths and other EVM
   opcodes; define stable exception codes; later address EVM memory, storage,
   gas, control flow, and broader exception semantics.
4. **Optional optimizations:** Profile the guest wrappers and termination
   paths; investigate native RV64 or another backend only with explicit
   metric-comparability work.

## Final recommendation

Proceed with the next narrow EVM-opcode milestone: the implemented initial
ABI and generated execution path are sufficiently validated, and no confirmed
ADR-001 defect blocks progress. Before substantially expanding opcode
coverage, prioritize F-001 so that RC whole-program execution—not only the
legacy Rust-managed `ProgramRunner` and arithmetic cores—has systematic REVM
differential evidence. Resolve the benchmark-artifact discrepancy before
using the runbook as archival reproducibility evidence.

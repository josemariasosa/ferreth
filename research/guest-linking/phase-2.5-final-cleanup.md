# Phase 2.5: Legacy Linker Removal and Final Cleanup

## 1. Baseline

- Branch `feat/symbolic-guest-composer`, HEAD `3eb4a94` (`feat(compiler): complete symbolic RV64 public cutover`).
- Worktree was clean before this phase. Nothing was committed, branch-switched, reset, or cleaned.

## 2. Production path

`compiler::compile_evm_bytecode` and `compiler::compile_push1_program` route only through
`symbolic::compile` and `symbolic::compile_push1_sequence`
([compiler.rs](/Users/valkyrie/eth/ferreth/crates/ferreth-compiler/src/compiler.rs),
[symbolic.rs](/Users/valkyrie/eth/ferreth/crates/ferreth-compiler/src/compiler/symbolic.rs)).

## 3. Legacy linking inventory

All whole-program legacy linkers are `#[cfg(test)]` and are not reachable from any production entry point:

- `compiler.rs`: `compile_evm_bytecode_legacy`, `compile_push1_program_legacy`,
  `compile_supported_program_legacy`, `CompiledOperation`, `GeneratedOperation`,
  `linked_{dup1,pop,add,sub}_assembly`, `relative_jal_offset`, `encode_jal_zero`,
  `instruction_at`, `instruction_count`.
- `orchestrator.rs`: `link_guest_add`, `link_guest_sub`, `link_guest_arithmetic`,
  `arithmetic_{normal,underflow}_exit_assembly`, and duplicate test-only JAL/instruction helpers.

`ProgramRunner` in `execution.rs` is a host arithmetic runner used by conformance. It does not link
guest images and is out of scope.

## 4. Changes made

1. [compiler.rs](/Users/valkyrie/eth/ferreth/crates/ferreth-compiler/src/compiler.rs): removed the stale
   `#[allow(dead_code, reason = "Phase 2.2 candidate awaits public routing review")]` on `pub(crate) mod symbolic`.
   A non-test build produced no warnings without it, so the allowance was obsolete.
2. [verify.rs](/Users/valkyrie/eth/ferreth/crates/ferreth-compiler/src/rv64/verify.rs): replaced the enum-wide
   `#[allow(dead_code, reason = "Phase 2.4 legacy-layout and rollback coverage")]` on `Fragment` and
   `PublisherLayout` with variant-level allowances. A non-test probe showed that only
   `Fragment::Arithmetic` and `PublisherLayout::{PushOnly, Mixed, IsolatedArithmetic}` are never constructed
   outside tests. Each allowance now carries the reason
   "constructed only by the test-only legacy linker oracles". All other variants stay under lint.
3. [compiler.rs](/Users/valkyrie/eth/ferreth/crates/ferreth-compiler/src/compiler.rs): corrected the
   `compile_push1_program` Rustdoc. It described `generate_guest_push1` bodies and patched trampolines,
   which are the retired mechanism. It now describes symbolic fragments linked by the single composer.

No production logic changed. No dependencies, opcodes, gas model, ADRs, or Phase 3 work were touched.

## 5. Test-oracle disposition

The legacy linkers are retained as isolated, test-only oracles. Replacing them with immutable fixtures would
rewrite dozens of whole-image comparisons, which is a large, risky change outside this cleanup. The brief allows
retention when safe replacement is not feasible.

| Oracle | Consumers | Independent property checked |
|---|---|---|
| `compile_evm_bytecode_legacy`, `compile_push1_program_legacy` | `compiler/symbolic/tests.rs`, `tests/rv64_push1_program.rs` | Symbolic composer output agrees with the independent string-patching linker |
| `compile_supported_program_legacy` | `tests/pop_dup1.rs`, `tests/arithmetic.rs` | Multi-opcode layouts match an independently assembled image |
| `link_guest_arithmetic` and helpers | `orchestrator.rs` test `isolated_arithmetic_images_match_legacy_singleton_linkers` | Isolated arithmetic images match legacy singleton linkers |
| Legacy layouts in `verify.rs` | `rv64/verify/tests.rs` | Verifier classifies both legacy and symbolic image layouts |

Technical debt recorded, not fixed here: the test-only JAL encoders and instruction counters are duplicated
between `compiler.rs` and `orchestrator.rs`.

## 6. Validation

Run with `CARGO_TARGET_DIR=target/phase-2.5-validation`. Logs are in `target/phase-2.5-validation/logs/`.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (after one `cargo fmt --all` to wrap the new attributes) |
| `cargo clippy --frozen --workspace --all-targets -- -D warnings` | pass, 0 warnings |
| `cargo test --frozen -p ferreth-compiler --lib` | pass |
| `cargo test --frozen -p ferreth-compiler --test rv64_push1_program` | pass |
| `cargo test --frozen -p ferreth-conformance --test rc_revm_conformance` | pass |
| `cargo test --frozen -p ferreth-linking-spike` | pass |
| `cargo test --frozen --workspace -- --list` | 215 tests (baseline 215) |
| `cargo test --frozen --workspace --no-fail-fast` | 212 passed, 0 failed, 3 ignored (baseline 212/3) |
| Capacity probes (lib `capacity`, `capacity_probe`) | 2 passed and 1 passed |
| `bench guest --format json` | SHA-256 `5fe319d3b352fd0e800decf469b8b17553df9c3848359a9f77f0d363a1c123af` (matches) |
| `bench --op ALL --cases 1000 --seed 42 --format json` | SHA-256 `f4f44b247a0d506779e306b2956d147ba4a216d67a8293295f641660a64c9f71` (matches) |

## 7. Limitations

- Legacy oracles remain in the test build. Full removal requires replacing them with immutable fixtures,
  which is deferred.
- The duplicated test-only helpers remain (see section 5).
- Only the workspace suites and the listed probes were run. No new differential conformance cases were added,
  because this phase changes no behavior.

## 8. Pending decisions for the maintainer

- Whether to replace the legacy oracles with immutable fixtures in a follow-up phase.
- Whether to consolidate the duplicated test-only JAL helpers.

## 9. Next logical action

Maintainer review of this report and [human-led-walk.md](./human-led-walk.md). No commit has been made.

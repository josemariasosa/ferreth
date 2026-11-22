# RV64 guest linking Phase 1

This crate hosts the planned symbolic assembly spike. Phase 1.0 establishes
an isolated workspace member with a doc-only library and a single dependency,
`asm-rs = "=0.2.0"`, with default features disabled and only `std` and
`riscv` requested. The crate is outside the workspace default members.

The spike will investigate whether symbolic assembly can produce the final
bytes and label addresses needed to compose and independently verify guests
under the accepted ABI. Phase 0 supplies the control artifacts and the four
failing 47-PUSH regressions. See the [incremental plan](PLAN.md),
[Phase 0 evidence](../README.md),
[guest linking research](../ferreth-linking-architecture.md),
and [ADR-001](../../../docs/adr/001-evm-guest-state-abi.md).

## Reproduce Phase 1.0 verification

Use Cargo and rustc 1.98.1, the cached workspace dependencies, and a fresh
snapshot and target directory. From the repository root with the scaffold
present, archive the tracked baseline and overlay the Phase 1.0 changes.
This follows the isolation used by the
[Phase 0 test replay](../phase-0.4-0.5/reproduce-tests.sh).

```sh
repo="$PWD"
out="$(mktemp -d /tmp/ferreth-linking-phase-1.XXXXXX)"
mkdir "$out/checkout"
git archive HEAD | tar -xf - -C "$out/checkout"
cp "$repo/Cargo.toml" "$repo/Cargo.lock" "$out/checkout/"
cp -R "$repo/research/guest-linking/phase-1" "$out/checkout/research/guest-linking/"
cd "$out/checkout"
export CARGO_TARGET_DIR="$out/target"
cargo check --frozen -p ferreth-linking-spike
cargo build --frozen -p ferreth-linking-spike
cargo tree --frozen -p ferreth-linking-spike -e features
cargo fmt --all -- --check
cargo clippy --frozen -p ferreth-linking-spike --all-targets -- -D warnings
cargo test --frozen --workspace --no-fail-fast -- --test-threads=1
```

Review `git diff HEAD -- Cargo.toml Cargo.lock` from the original checkout.
The manifest diff should add only the workspace member and the lockfile diff
should add only the local spike package entry.

## Phase 1.0 results

The isolated verification on `aarch64-apple-darwin` used baseline
`587dbcd67dfa8e57251c236bd0f44d1b0c31a3e1`. Frozen check, build, feature
resolution, formatting and spike Clippy checks passed. Cargo resolved only
`asm-rs 0.2.0` for the spike, with `std` and `riscv`. The lockfile matched
the baseline byte for byte after removing the new local package entry.

The complete workspace run exited 101 with 141 tests, 137 passed, 4 failed
and 0 ignored. All four failures matched the [Phase 0 inventory](../../../runs/phase-0/workspace-verification.json).
Both 47-PUSH EOF regressions rejected a numeric JAL displacement of +2056.
Both explicit STOP regressions rejected +2060. These are baseline compiler
failures. The scaffold build and test run establish package isolation and
baseline preservation. Assembler behavior and symbolic guest execution
remain pending.

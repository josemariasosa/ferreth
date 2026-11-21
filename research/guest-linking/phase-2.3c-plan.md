# Phase 2.3c plan — Symbolic ADD and SUB integration

**Status: Proposed for maintainer review.** This plan concerns the private
symbolic compiler under accepted ADR-001. Implementation requires separate
authorization. Public whole-program and isolated arithmetic entry points
stay on their existing routes through this phase.
[ADR-001](../../docs/adr/001-evm-guest-state-abi.md) · [Accepted Phase 2.3b.1](phase-2.3b.1-boundary-hardening.md)

## Fragment and verifier contract

1. Add private ADD and SUB lowering through the Phase 2.3a fragment contract.
   Preserve each existing 43-word layout: nine wrapper words followed by
   one occurrence-local 34-word leaf. Reuse the established core source.
   Keep the arithmetic pointer ABI: `a0 = s1 - 32` and
   `a1 = a2 = s1 - 64`. Underflow at depths zero and one publishes code 2
   before any operand access. After a successful call the wrapper retires
   the top slot exactly once.
2. Give fragment entries, underflow trampolines and leaves deterministic
   occurrence-local labels. The continuation names the next fragment,
   explicit STOP or the Normal publisher at EOF. Declare the local depth
   branch, `jal ra` call, return, continuation JAL and external Underflow
   JAL. Retain declaration-derived publisher inventory and the shared
   publisher source. Keep opcode dispatch in the private frontend and
   generator-specific layout in fragment generation.
3. Extend the declaration scan only as needed to verify the existing
   `jalr x0, 0(ra)` leaf return. Check the exact return word and its
   relationship to the associated in-image call: the call links to `ra`,
   targets that occurrence's own leaf, and `call_pc + 4` identifies its
   `addi s1, s1, -32`. A return target comes from that verified call
   relationship. The independent fixed-layout check continues to require
   the call at word 5, stack decrement at 6, continuation at 7, underflow
   trampoline at 8, leaf entry at 9 and return at 42. The composer
   continues to place fragments and resolve symbols without opcode-specific
   linking. Keep indirect transfer support limited to this leaf return.

[Fragment declarations](phase-2.3a-fragment-contract.md) · [Existing arithmetic layout checks](../../crates/ferreth-compiler/src/rv64/verify.rs) · [Phase 1 mixed composition evidence](phase-1/results/1.6-mixed-symbolic-guest.md)

## Verification and acceptance

- Replay the 17 Phase 1.6 specimens through the private frontend and public
  legacy path with identical input and seeds. Add focused ADD and SUB
  depth-zero, depth-one and depth-two cases, multi-limb carry and borrow,
  explicit SUB operand ordering, wraparound, repeated calls to distinct
  leaves, EOF and STOP with unreachable trailing bytecode. Check independent
  U256 expectations, complete live stacks, stack guards and live and
  published `s1`. Verify `s0` and native `sp` preservation, the expected
  call writes to `ra`, publication store order and sentinel termination.
  The established body counts are 42 instructions per successful arithmetic
  operation and 3 on underflow. Normal and exceptional publishers take 7 and
  8 instructions respectively.
- Compare dynamic outcomes and record static publisher inventory, PCs,
  body bytes and metrics by route. For arithmetic-only guests with Normal
  at `P`, expect candidate Underflow at `P+28`, and legacy Overflow then
  Underflow at `P+28` and `P+60`. When Overflow is required by PUSH1 or
  DUP1, expect candidate Underflow then Overflow, with sizes checked
  independently. Preserve each embedded leaf's byte equality with its
  standalone core, including the 34-word sizes and existing FNV-1a hashes
  (`0xf1fff32ff47962f3` for ADD and `0x9f008f320eeb7a7b` for SUB).
- Reject a cross-occurrence call, wrong call link register, altered
  return register or immediate, wrong return anchor, misdirected
  continuation or underflow, and missing, duplicate or unexpected
  transfers. Include a case where changed bytes and declarations agree
  but violate the fixed arithmetic layout. Preserve all prior negative
  fixtures and check that the final image still reassembles as displayed.
- Retain the Phase 2.3b.1 opt-in capacity probes and controlled link/layout
  errors. RF-019 is closed and RF-020 is the accepted implementation
  limit. Carry forward the current label density, resource limits and
  ADR-001 bound. Keep arithmetic cores, executor behavior, ABI and gas
  behavior fixed.
- After authorized implementation, run focused candidate and verifier
  tests, `cargo fmt --all -- --check`,
  `cargo clippy --frozen --workspace --all-targets -- -D warnings`,
  `cargo test --frozen -p ferreth-linking-spike`,
  `cargo test --frozen --workspace -- --list` and
  `cargo test --frozen --workspace --no-fail-fast`. Record exact totals,
  opt-in probe results and the standalone arithmetic benchmark against
  the approved baseline. The Phase 2.3b.1 baseline is 193 discovered,
  186 passing, 4 failing and 3 ignored. The only expected failures are
  the four public LF-026 PUSH1 controls. A changed leaf, incorrect
  transfer, unexplained runtime delta or new test failure blocks
  acceptance. Stop at private integration review before public cutover.

[Phase 1.6 specimen and core results](phase-1/results/1.6-mixed-symbolic-guest.md) · [Core benchmark runbook](../../docs/runbooks/benchmark.md)

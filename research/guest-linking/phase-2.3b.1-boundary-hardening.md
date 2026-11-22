# Phase 2.3b.1 — POP/DUP1 boundary and capacity hardening

## Review status and reproduction basis

**Status: Reviewed and accepted under the maintainer's capacity policy.**
RF-019 is closed. RF-020 records a known implementation capacity limit.
Phase 2.3c is authorized for planning.

The initial branch was `feat/symbolic-guest-composer` at
`da505fcc5edab4f78f8e06e3de3bb5fc7029b737`. The initial result of
`git status --short --branch` was only `## feat/symbolic-guest-composer`.
The worktree and index were clean. The runs used Cargo/rustc 1.98.1 on
`aarch64-apple-darwin`, frozen `asm-rs 0.2.0` and `rvemu 0.0.11`.
The directed comparisons use supported stack operations with the same
bytecode, seeded state and [accepted ADR-001](../../docs/adr/001-evm-guest-state-abi.md)
execution conditions on both paths. POP/DUP1 stack semantics are common to the
project's Cancun reference fork. This phase compares Ferreth paths directly.
The workspace retains the existing REVM 43.0.3 Cancun conformance tests.

## RF-019 — DUP1 JAL range rejection occurs in oversized guests

The [Phase 2.3b report](phase-2.3b-pop-dup1.md#9-popdup1-label-density-and-capacity)
recorded errors targeting Underflow. For both 33,331 and 33,332 DUP1s the source
span points to line 17, column 5, byte offset 328, at
`jal zero, binary_stack_underflow_exit` immediately after `dup1_underflow_0:`.
This is the J-type `JAL x0` at word 14 in `guest_dup1_0`. The probe checks the
reported span against emitted source. The rejected guests have no final
image. The reported displacement is observed, and the PCs below are derived
from the fixed layout and confirmed against the same labels on assembling
control images.

| DUP1s at EOF | Source bytes | Transfer site and target | Displacement | Outcome and image bytes |
|---:|---:|---|---:|---|
| 16,382 | 7,256,629 | Both first exceptional JALs resolve | +1,048,420 Underflow, +1,048,448 Overflow | 1,048,540 bytes, assembled and verified with 49,149 labels |
| 16,383 | 7,257,076 | Both first exceptional JALs resolve | +1,048,484 Underflow, +1,048,512 Overflow | 1,048,604 bytes, assembled with 49,152 labels and rejected by layout |
| 16,384 | 7,257,523 | `dup1_overflow_0` `0x8000003c` → `push1_stack_overflow_exit` `0x8010003c` | +1,048,576 | First JAL range rejection for pure DUP1 EOF. Nominal image 1,048,668 bytes |
| 33,331 | 14,832,832 | `dup1_underflow_0` `0x80000038` → `binary_stack_underflow_exit` `0x80208cdc` | +2,133,156 | JAL range rejection. Nominal image 2,133,276 bytes |
| 33,332 | 14,833,279 | `dup1_underflow_0` `0x80000038` → `binary_stack_underflow_exit` `0x80208d1c` | +2,133,220 | JAL range rejection. Nominal image 2,133,340 bytes |
| 33,333 | 14,833,726 | Label processing fails before relocation | — | 100,002 nominal labels against a limit of 100,000. Nominal image 2,133,404 bytes |

The run measured 14,833,279 source bytes for 33,332 DUP1s. The Phase 2.3b table
records 14,832,279. The complete raw assembler diagnostics for the two
original range probes are:

```text
BranchOutOfRange { label: "binary_stack_underflow_exit", disp: 2133156, max: 1048574, span: Span { line: 17, col: 5, offset: 328, len: 3 } }
BranchOutOfRange { label: "binary_stack_underflow_exit", disp: 2133220, max: 1048574, span: Span { line: 17, col: 5, offset: 328, len: 3 } }
```

For `N` DUP1s at EOF, each body is 64 bytes and the publishers occupy 92 bytes
in Normal/Underflow/Overflow order. The first Underflow JAL uses displacement
`(64*N + 28) - 56`. The corresponding Overflow JAL uses
`(64*N + 60) - 60`. Subsequent JAL sites are closer to their publishers.
The local B-type `bgeu` instructions reach their same-fragment trampolines
at fixed displacements +56 (Underflow) and +48 (Overflow).

The ISA's B-type range is −4,096 through +4,094 bytes, and the J-type JAL
range is −1,048,576 through +1,048,574 bytes, both in two-byte increments.
In pinned `asm-rs 0.2.0`, `src/linker.rs` applies `RvJal20` by computing
`target_addr - reloc_abs + addend`, with addend zero here. It checks the
signed interval `[-2^20, 2^20)` and then enforces the two-byte scale.
`RvBranch12` uses `[-2^12, 2^12)`. Both paths share the diagnostic variant
`BranchOutOfRange`. The raw `max: 1048574` identifies the JAL check in these
cases. [RISC-V unprivileged ISA, control transfer instructions](https://docs.riscv.org/reference/isa/unpriv/rv32.html#_control_transfer_instructions)

ADR-001 reserves the exit slot and admits at most 1,048,572 executable bytes.
Two complete, four-byte-aligned instructions inside such an image have at
most 1,048,568 bytes of separation. Every internal JAL in the accepted
PUSH1/STOP/POP/DUP1 layout therefore fits in either direction. The local
conditionals are separately bounded by their fixed fragment layouts, with a
maximum of +56 bytes. At 16,382 DUP1s the probe executes both distant first
exceptional JALs. The empty seed reaches Underflow at `0x800fff9c` with 2 + 8
instructions. The full seed reaches Overflow at `0x800fffbc` with 5 + 8.
A one-item seed duplicates that word 1,023 times and then reaches Overflow
at the same PC with 14,327 + 8 instructions and all 1,024 words intact.

**Disposition:** Closed as an out-of-domain capacity observation.
The opt-in probe remains the regression for this conclusion, with current
branch lowering, relaxation policy and limits.

## RF-020 — POP label limit binds before the executable image limit

A nonempty POP-only EOF candidate uses 16 bytes and two labels per POP, plus
the 28-byte Normal and 32-byte Underflow publishers with one label each.
Thus `image_bytes = 16*N + 60` and `labels = 2*N + 2`.
`asm-rs 0.2.0` counts defined labels while processing statements and reports
`ResourceLimitExceeded { resource: "labels", limit: 100000 }` on the
first label above its default. Source-derived layout values below are nominal
where the assembler returns no bytes or label map.

| POPs at EOF | Source bytes measured | Labels | Image bytes | Assembler / image layout |
|---:|---:|---:|---:|---|
| 49,999 | 8,355,739 | 100,000 | 800,044 | Assembled and verified |
| 50,000 | 8,355,907 | 100,002 | 800,060 | Label-limit rejection. Derived size fits |
| 65,532 | 10,965,283 | 131,066 | 1,048,572 | Label-limit rejection. Size validator accepts a code length ending exactly at the exit slot |
| 65,533 | 10,965,451 | 131,068 | 1,048,588 | Label-limit rejection. Size validator rejects a code end 16 bytes beyond the exit slot |

An explicit STOP adds one label and four bytes. By the same formulas, the
POP+STOP assembler boundary is 49,998 POPs (99,999 labels, 800,032 bytes)
followed by 49,999 POPs (100,001 labels, 800,048 bytes). The image-only
boundary is 65,531 POPs (1,048,560 bytes) followed by 65,532 POPs (1,048,576
bytes). These STOP thresholds are layout-derived. The large probes use EOF.
The pure POP image-only boundary cannot be reached through the default
assembler with this label representation. The probe checks it by passing
the derived lengths directly to `validate_generated_code`.

Runtime reachability is separately constrained by the initial depth.
The verified 49,999-POP image ran from 1,024 seeded words. It executed 1,024
successful POPs, then the next POP took the distant Underflow JAL to
`0x800c350c`. The stack was empty, the failing POP left the backing memory
and guard unchanged, and the run took 3,074 + 8 instructions. Later bodies
were unreachable. The guards and publication checks match the short cases.

**Disposition: Accepted as a known implementation capacity limit.**
The default `asm-rs 0.2.0` resource limits and ADR-001's executable image
bound apply independently. A layout within the image bound can still exceed
the assembler's limit. The 49,999/50,000 POP boundary describes the current
fragment representation. EVM semantics and Ethereum consensus impose no
such POP count. Retain the opt-in capacity probe and controlled compilation
errors for resource and layout violations. Keep the current label density
and limits. The existing probes serve as the capacity regression evidence
for later phases. This decision resolves the POP capacity-policy question in
[RF-016 and the Phase 2 readiness stop conditions](phase-2.0-readiness.md#8-baseline-verification-and-stop-conditions).

## RF-021 — Stack boundaries and ADR-001 publication agree on both routes

All 16 directed vectors passed on both private symbolic `compile` and public
legacy `compile_evm_bytecode`. They use deterministic full-width U256
seeds, and both routes are checked against independently specified results.
In the table, `W[i]` is seeded word `i` with indices from the bottom.
An initial depth `d` uses `W[..d]` unless otherwise specified. Counts are
body plus publisher instructions. STOP contributes one body instruction
only when reached. The sentinel contributes zero.

| Input | Initial depth | Expected live stack | Status / code | Final `s1` | RV64 body + publisher |
|---|---:|---|---|---|---:|
| POP | 0 | Empty | Underflow / 2 | `0x80101000` | 2 + 8 |
| POP | 1 | Empty | Normal / 0 | `0x80101000` | 3 + 7 |
| POP | 2 | `W[..1]` | Normal / 0 | `0x80101020` | 3 + 7 |
| POP | 1,024 | `W[..1023]` | Normal / 0 | `0x80108fe0` | 3 + 7 |
| POP × 2 | 2 | Empty | Normal / 0 | `0x80101000` | 6 + 7 |
| POP × 3 | 2 | Empty | Underflow / 2 | `0x80101000` | 8 + 8 |
| DUP1 | 0 | Empty | Underflow / 2 | `0x80101000` | 2 + 8 |
| DUP1 | 1 | `W[..1]` + `W[0]` | Normal / 0 | `0x80101040` | 14 + 7 |
| DUP1 | 1,022 | `W[..1022]` + `W[1021]` | Normal / 0 | `0x80108fe0` | 14 + 7 |
| DUP1 | 1,023 | `W[..1023]` + `W[1022]` | Normal / 0 | `0x80109000` | 14 + 7 |
| DUP1 | 1,024 | `W[..1024]` | Overflow / 1 | `0x80109000` | 5 + 8 |
| DUP1 × 2 | 1,022 | `W[..1022]` + two copies of `W[1021]` | Normal / 0 | `0x80109000` | 28 + 7 |
| DUP1 × 3 | 1,022 | `W[..1022]` + two copies of `W[1021]` | Overflow / 1 | `0x80109000` | 33 + 8 |
| `6007508060085080005080` | 2 | `[W[0], W[1], W[1], W[1]]` | Normal / 0 | `0x80101080` | 55 + 7 |
| `60098050508000` | 0 | Empty | Underflow / 2 | `0x80101000` | 32 + 8 |
| `50600950808000` | 1,024 | `W[..1023]` + `W[1022]` | Overflow / 1 | `0x80109000` | 35 + 8 |

The first mixed vector contains repeated PUSH1, POP and DUP1 fragments.
Each compiler produces its own identical artifact when the trailing POP
and DUP1 after STOP are removed, and the untrimmed bytecode passes the
runtime checks. In the exceptional runs the harness single-steps the actual
fault path at expected PCs and checks each executed instruction opcode.
POP and empty DUP1 take a B-type test and a JAL. Overflowing DUP1 executes
the two guards, limit calculation and overflow JAL. These paths contain no
loads or stores. A snapshot of the complete 32-KiB stack allocation plus
guard before and after the failing opcode independently checks memory
preservation, including deallocated words after POP exhaustion.

Every vector checks terminal status, exception code, the complete live
stack, live and published `s1`, `s0`, native `sp`, `ra`, the EVM stack
guard and a 64-byte native-stack canary around `sp`. The test observes
the 24-byte metadata before publication and after every publisher
instruction. Literal store words ensure the order even when a store leaves
bytes unchanged. The order is `final_s1`, `exception_code`, `status`, then
the sentinel, observed before fetch. Underflow and Overflow both publish
ADR-001 `GuestException` with their distinct provisional codes.

### Parity and static layout

Both routes yield the same results, publisher byte traces and dynamic
counts on the 16 vectors. At the same Normal PC `P`, legacy always has
Normal, Overflow, Underflow at `P`, `P+28`, `P+60`. Symbolic POP-only
has Normal and Underflow at `P`, `P+28`, saving the 32-byte unused Overflow
publisher. For the tested DUP1 and mixed inputs, symbolic has Normal,
Underflow, Overflow at `P`, `P+28`, `P+60`. Total size then agrees and the
two exceptional roles exchange PCs, so their JAL encodings can differ.
Semantic results and metadata are compared directly across paths.
Publisher PCs are asserted against the specific layout of each path.

Capacity-scale parity is unavailable through the public route. Bounded
legacy probes assembled 124 POPs and 31 DUP1s at EOF but reproduced the
numeric operand parser's `ImmediateOverflow` for 125 POPs (+2,048) and
32 DUP1s (+2,052), with parser bounds −2,048 through +2,047. The
capacity-scale symbolic runs are verified against independent expectations.
The small directed comparison, large symbolic probes and existing workspace
suite are the limits of the runtime evidence.

### Independent final-image verification

The existing declaration scanner and fixed-layout completeness checks were
both exercised. The retained tests reject missing or duplicate declarations,
wrong local targets, undeclared control transfers, corrupted JAL/branch
encodings, unexpected relaxation and missing publishers. The new symbolic
DUP1→POP negative control swaps the Underflow and Overflow JAL targets.
It first confirms the original declarations reject that image. It then
swaps the associated declaration roles to agree with the mutated bytes and
confirms the independent fixed-layout check still rejects the first JAL at
`0x80000038`, which targets `0x8000008c` instead of `0x8000006c`.
The verifier implementation and previous negative fixtures are intact.

## Verification commands and results

| Command | Result |
|---|---|
| `cargo test --frozen -p ferreth-compiler --lib compiler::symbolic::tests` | 12 passed, 2 ignored, 59 filtered |
| `cargo test --frozen -p ferreth-compiler --lib rv64::verify::tests` | 13 passed, 60 filtered |
| `cargo test --frozen -p ferreth-compiler --lib pop_dup1_capacity_probe -- --ignored --nocapture --test-threads=1` | 1 passed, 72 filtered |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --frozen --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --frozen -p ferreth-linking-spike` | 22 passed, 0 failed, 1 ignored |
| `cargo test --frozen --workspace -- --list` | 193 tests, 0 benchmarks |
| `cargo test --frozen --workspace --no-fail-fast` | 186 passed, 4 failed, 3 ignored, 0 measured, 0 filtered. Exit 101 |
| `cargo test --frozen -p ferreth-compiler --lib push1_capacity_boundary -- --ignored --nocapture --test-threads=1` | 1 passed, 72 filtered |

The full workspace ran 190 tests and discovered all four existing public
LF-026 regressions in `rv64_push1_program`. They were its sole failures:

| Test | Legacy assembler error |
|---|---|
| `repeated_push1_47_eof_completes_normally` | `ImmediateOverflow` at +2,056 |
| `repeated_push1_47_eof_full_stack_publishes_overflow` | `ImmediateOverflow` at +2,056 |
| `repeated_push1_47_stop_completes_normally` | `ImmediateOverflow` at +2,060 |
| `repeated_push1_47_stop_full_stack_publishes_overflow` | `ImmediateOverflow` at +2,060 |

All four failed at line 12, column 5 during legacy display-assembly
reassembly, with bounds −2,048 through +2,047. The normal run's three
ignored tests are the POP/DUP1, PUSH1 and research spike capacity probes.
The two private probes passed as explicit opt-ins. The research spike's
ignored probe was left opt-in. With captured output disabled, rvemu
emitted device-tree warnings on this host. Those runs completed in the
ADR-001 memory regions and their assertions passed.

## Changed files

| File | Change |
|---|---|
| [`crates/ferreth-compiler/src/compiler/symbolic/tests.rs`](../../crates/ferreth-compiler/src/compiler/symbolic/tests.rs) | Shared publisher stepping and independent store checks. The focused POP/DUP1 runtime and capacity cases move to the dedicated test module. |
| [`crates/ferreth-compiler/src/compiler/symbolic/tests/pop_dup1.rs`](../../crates/ferreth-compiler/src/compiler/symbolic/tests/pop_dup1.rs) | Directed runtime and differential tests, symbolic fixed-layout role-swap rejection, and opt-in capacity probes. |
| [`research/guest-linking/phase-2.3b.1-boundary-hardening.md`](phase-2.3b.1-boundary-hardening.md) | Evidence and review decision. |
| [`research/guest-linking/README.md`](README.md) | Research index entry. |
| [`research/guest-linking/ferreth-linking-architecture.md`](ferreth-linking-architecture.md) | Phase 2 roadmap status and review gate. |

## Maintainer acceptance

The maintainer reviewed and accepted Phase 2.3b.1 on 2026-11-20 under the
capacity policy above. The existing probes carry forward as regressions
through the [Phase 2.3c integration plan](phase-2.3c-plan.md).
The next authorization in scope is planning. Implementation awaits separate
maintainer authorization.

# Phase 2.2 — Private symbolic PUSH1 and STOP composer

The implementation is on `feat/symbolic-guest-composer`, based on
`b5029b23ab1823c86f3e3eb898cf14c74b0989d2`. This phase asks whether the
Phase 1 PUSH1 and STOP layout can return the existing production artifact
with independently verified control flow, ADR-001 execution parity and
the measured near-boundary image capacity. Passing requires observing the
first distant overflow jump on a seeded full stack as well as normal
execution. The current public routes provide the legacy comparison.
[ADR-001](../../docs/adr/001-evm-guest-state-abi.md) ·
[RF-012](phase-1/results/1.4-symbolic-push1.md) ·
[RF-013](phase-1/results/1.5-long-range-guest.md) ·
[RF-015](phase-1/results/1.7-architecture-decision.md) ·
[RF-017](phase-2.1-final-image-verification.md)

## RF-018 — Symbolic composition verifies and executes the PUSH1 and STOP guest

### Internal API and ownership

`compiler::symbolic::compile(&[u8])` uses `decode_detailed` and lowers the
execution prefix to `SymbolicFragment::Push1(u8)` and `SymbolicFragment::Stop`.
The decoder supplies the zero immediate for truncated `60` and stops at
the first `00`. The candidate maps decoded instructions outside this
subset to the existing unsupported-instruction error with the byte offset.
`compose(&[SymbolicFragment]) -> Result<CompiledRv64Program, CompileError>`
also accepts a direct fragment sequence. An empty fragment sequence falls
through to normal publication, as does the last successful PUSH1 at EOF.
The private module compiles in the ordinary library, with the test module
exercising it before public routing is enabled.

The composer assigns each PUSH1 a unique occurrence index. The `guest_push1_`,
`push1_overflow_` and `push1_continue_` label families have distinct names
and are separate from STOP and publisher names. The shared PUSH1 body
emitter supplies the unchanged stack check, four limb stores and pointer
advance. The symbolic emitter names the local branch and continuation and
takes the overflow publisher symbol as an operand. STOP emits one symbolic
`jal zero` to normal publication. The standalone fragment still emits its
original numeric local operands and final placeholder, with the same
public type and entry point.

`compiler::publisher::Publisher` owns the compiler's ABI source. The enum
maps semantic roles to reserved names. Public routes use that source with
their existing inventories. The candidate emits normal and overflow in
that order. The generated code derives metadata from `s0`, stores `final_s1`
at byte 16, writes exception code at byte 8, then publishes status at byte 0.
Both candidate publishers use `jalr zero, -4(t1)` to the reserved sentinel.
The existing isolated arithmetic publisher stays in the orchestrator.

### Independent verification

The composer uses one `Assembler::emit` and `finish` at `ENTRY_PC` with
locked `asm-rs 0.2.0` and default resource limits. It validates the result
origin, four-byte length and executable interval, then resolves the two
publisher PCs through the final label map. The ordered fragment kinds are
the compile-time declaration of the complete set of sites. Each PUSH1
requires a branch at word 2 to local word 10, a `jal x0` at word 9 to the
next fragment or publisher, and a `jal x0` at word 10 to Overflow. STOP
requires `jal x0` at word 0 to the immediately following Normal publisher.
The verifier obtains local anchors from this fixed-width contract. Its
independent decoder obtains actual targets and instruction fields from
the little-endian machine words. It checks both publisher PCs against the
fragment widths, verifies the complete ABI bodies and checks full image
consumption. It uses no assembler relocation records. All fragment words
are scanned, so a missing or extra transfer, changed width, or relaxed
branch that inserts another instruction fails the contract.

The candidate mutation test assembles images with wrong overflow and STOP
roles, an omitted or misdirected continuation, an expanded local branch,
and a missing publisher label. All are rejected, as are an omitted STOP
declaration and a direct fragment sequence with a nonterminal STOP. The
production composer returns the verified bytes, assembly, publisher PCs
and static counts in `CompiledRv64Program`. Reassembly at the guest origin
is a test of the returned presentation source.

### Observed parity and execution

Empty, one-, two- and five-PUSH1 EOF programs matched legacy bytes exactly,
including `60026003` at 148 bytes. The truncated `60` matched `6000` in both
implementations. First STOP ignored an invalid opcode and a truncated PUSH
after STOP. The plain STOP and PUSH1/STOP candidate artifacts matched the
entire reachable legacy byte prefix, including both publisher bodies and
PCs. As in RF-013, each candidate STOP image is 32 bytes smaller because
the legacy explicit-STOP path also appends an unreachable underflow
publisher. Symbolic display operands differ on the candidate. The retained
public path is the compatibility boundary for those deltas.

For each legacy comparison, the tests compare the reached publisher and
sentinel, full guest results and raw metadata bytes before and after each
publisher instruction, and RV64 body and publisher counts. Each run also
asserts ABI `s0`, live `s1`, native `sp` and the 32-byte `0xa5` guard above
the stack. A trace can contain duplicate states when a store writes the
value already present. The verifier separately checks that store's actual
instruction word. In the seeded depth-1,023 control, the first PUSH1 writes
255 to the final usable slot, the second fails, and the RV64 counts are
14 body plus 8 publisher instructions in both implementations.

| Input | Candidate bytes | Normal PC | Overflow PC | First overflow JAL word / displacement | Empty stack RV64 body + publisher | Seed-42 full stack RV64 body + publisher |
| --- | ---: | --- | --- | --- | ---: | ---: |
| 46 × `6002`, EOF | 2,084 | `0x800007e8` | `0x80000804` | `0x7dc0006f` / +2012 | 460 + 7 | 4 + 8 |
| 46 × `6002`, STOP | 2,088 | `0x800007ec` | `0x80000808` | `0x7e00006f` / +2016 | 461 + 7 | 4 + 8 |
| 47 × `6002`, EOF | 2,128 | `0x80000814` | `0x80000830` | `0x0090006f` / +2056 | 470 + 7 | 4 + 8 |
| 47 × `6002`, STOP | 2,132 | `0x80000818` | `0x80000834` | `0x00d0006f` / +2060 | 471 + 7 | 4 + 8 |

The four empty-stack runs published status 1 and code 0 with the full
stack of twos. Live and published `s1` were `0x801015c0` at 46 pushes and
`0x801015e0` at 47. The four seeded runs reached the first trampoline at
`0x80000028` after three instructions, before the first store path. One
more instruction arrived at the independently checked overflow PC shown
above. Each published status 2, code 1 and `s1=0x80109000`, with all 1,024
seeded words preserved. Each run reached `0x800ffffc` before fetching the
EBREAK planted there. The two 46-PUSH layouts matched their legacy runtime
controls. Legacy assembly rejected the 47-PUSH layouts before execution.
The literal words, publisher PCs and run outcomes come from the independent
RF-013 fixtures for those layouts. The archived EELS Cancun and direct
REVM Cancun comparisons cover empty-stack termination and final stack.
The full-stack checks exercise ADR-001 and the Phase 1 RV64 reference.
[Phase 1.5 fixtures](phase-1/tests/long_range_guest.rs) ·
[Reference capture](README.md#phases-04-and-05-boundary-and-regression-capture)

### Capacity under default limits

The ignored production test emitted the actual candidate source, assembled
it at the guest origin, counted the final labels and sent the same result
through the production artifact check. All four inputs used `6002` repeated
as specified, optionally followed by STOP. This measures assembly and
layout acceptance. Runtime tests use the shorter inputs above.

| PUSH1 count | STOP | Source bytes | Final labels | Assembler | Image bytes | Ferreth layout |
| ---: | --- | ---: | ---: | --- | ---: | --- |
| 23,829 | EOF | 7,713,049 | 71,489 | Success | 1,048,536 | Accepted |
| 23,830 | EOF | 7,713,375 | 71,492 | Success | 1,048,580 | `CodeOverlapsExitSlot` |
| 23,829 | Explicit | 7,713,087 | 71,490 | Success | 1,048,540 | Accepted |
| 23,830 | Explicit | 7,713,413 | 71,493 | Success | 1,048,584 | `CodeOverlapsExitSlot` |

The observed images follow 44 bytes and three labels per PUSH1, 60 bytes
and two labels for the two publishers, and four bytes and one label for
explicit STOP. Ferreth's 1,048,572-byte executable limit binds first here.
The accepted EOF and STOP images leave 36 and 32 bytes before the sentinel,
respectively. RF-016 establishes POP's 16-byte body and the distinct
label-density problem for POP. This test establishes the PUSH1 case.
[Phase 2.0 capacity evidence](phase-2.0-readiness.md#guest-size-measurements)

### Frozen verification

The run used Cargo and rustc 1.98.1 on `aarch64-apple-darwin`,
`asm-rs 0.2.0` with `std` and `riscv`, `rvemu 0.0.11`, and REVM 43.0.3.
rvemu printed its existing missing-DTB warnings because `dtc` was
unavailable. An initial focused test build failed
because it used a runtime array repeat count for the publication trace.
The test now resizes the vector, and the recorded runs use this fix.

| Run | Cases | Passed | Failed | Ignored | Exit |
| --- | ---: | ---: | ---: | ---: | ---: |
| Compiler symbolic tests | 6 | 5 | 0 | 1 | 0 |
| Compiler package | 147 | 142 | 4 | 1 | 101 |
| Phase 1 research | 23 | 22 | 0 | 1 | 0 |
| Frozen workspace | 185 | 179 | 4 | 2 | 101 |
| Candidate capacity, explicit ignored-only rerun | 1 | 1 | 0 | 0 | 0 |
| Phase 1 capacity, explicit ignored-only rerun | 1 | 1 | 0 | 0 | 0 |

Frozen workspace discovery exited 0. All 185 target/name pairs matched
the execution log. There were zero measured or filtered workspace tests.
The two capacity tests were ignored by default. The explicit Phase 1
probe reproduced the 100,000-label default, rejection at 100,001 and
the established POP resource boundary. Workspace formatting and strict
frozen workspace Clippy passed.

| Unchanged LF-026 test | Legacy JAL displacement |
| --- | ---: |
| `repeated_push1_47_eof_completes_normally` | +2056 |
| `repeated_push1_47_eof_full_stack_publishes_overflow` | +2056 |
| `repeated_push1_47_stop_completes_normally` | +2060 |
| `repeated_push1_47_stop_full_stack_publishes_overflow` | +2060 |

Each failed during legacy display-assembly reassembly with
`ImmediateOverflow`, lower bound −2048, upper bound +2047, at line 12,
column 5. The test file retains Git blob
`821e0968cf47fa0c83edc05e2cbf0746e484de8e`.

For public-route compatibility, the same Phase 0.3 observer ran against
standalone frozen builds of the Phase 2.1 baseline and this phase. All
236 files from its 41 specimens and sequence-equality check matched by
name and bytes, as did the observer's two streams. The comparison covers
public assembly, machine code, publisher maps, publication traces and
runtime outcomes. The inventory SHA-256 was
`1b6f8769058be4280ed66125a0a41d21721b6ab9490e6d32977cbc7b8707d6c5`,
computed from the sorted `<sha256>  <filename>` lines with a newline after
each entry. The arithmetic benchmark matched raw stdout and stderr across
both snapshots, with 1,012 of 1,012 cases passed per operation and 34 static
and executed instructions per case. The ADD and SUB hashes were
`0xf1fff32ff47962f3` and `0x9f008f320eeb7a7b`. Raw stdout SHA-256 was
`f4f44b247a0d506779e306b2956d147ba4a216d67a8293295f641660a64c9f71`.
The JSON payload starts at byte 326, after the existing warnings. Raw
stderr was empty. RV64 counts measure completed guest instructions.
[Phase 0.3 observer](phase-0.3/observe.rs) ·
[Phase 2.1 replay recipe](phase-2.1-final-image-verification.md#reproduction)

### Reproduction

Use the frozen cache and a fresh snapshot. For a working tree based on the
Phase 2.1 commit above, overlay the Phase 2.2 compiler files as follows.
The snapshot keeps rvemu's DTS writes out of the working tree's sources.

```sh
mkdir -p target
out="$(mktemp -d "$PWD/target/phase22.XXXXXX")"
mkdir "$out/checkout"
git archive b5029b23ab1823c86f3e3eb898cf14c74b0989d2 | tar -xf - -C "$out/checkout"
cp crates/ferreth-compiler/src/compiler.rs "$out/checkout/crates/ferreth-compiler/src/"
cp -R crates/ferreth-compiler/src/compiler "$out/checkout/crates/ferreth-compiler/src/"
cp crates/ferreth-compiler/src/rv64/opcodes/push.rs "$out/checkout/crates/ferreth-compiler/src/rv64/opcodes/"
cd "$out/checkout"
export CARGO_TARGET_DIR="$out/build"
cargo test --frozen -p ferreth-compiler --lib compiler::symbolic::tests -- --nocapture --test-threads=1
cargo test --frozen -p ferreth-compiler --lib compiler::symbolic::tests::push1_capacity_boundary -- --ignored --exact --nocapture --test-threads=1
cargo fmt --all -- --check
cargo clippy --frozen --workspace --all-targets -- -D warnings
cargo test --frozen -p ferreth-compiler --no-fail-fast -- --test-threads=1
cargo test --frozen -p ferreth-linking-spike -- --test-threads=1
cargo test --frozen --workspace -- --list
cargo test --frozen --workspace --no-fail-fast -- --test-threads=1
cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1
cargo tree --frozen -e features -i asm-rs
cargo run --frozen -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json
```

For the public replay, also build an unmodified snapshot of the baseline
with a separate Cargo target. Use the Phase 2.1 observer recipe with each
build's own Cargo artifact records. The local run retains logs, observer
specimens and benchmark streams under `target/phase22.CSbOJg/`.

### Diff and exit gate

| File | Scoped change |
| --- | --- |
| [compiler.rs](../../crates/ferreth-compiler/src/compiler.rs) | Private module wiring and calls to the shared compiler publisher emitter. |
| [publisher.rs](../../crates/ferreth-compiler/src/compiler/publisher.rs) | One compiler-side source for publication instructions, status values and labels. |
| [symbolic.rs](../../crates/ferreth-compiler/src/compiler/symbolic.rs) | Candidate lowering, composition, assembly, verification and artifact construction. |
| [opcodes/push.rs](../../crates/ferreth-compiler/src/rv64/opcodes/push.rs) | Shared PUSH1 body emission and symbolic anchors. |
| [symbolic/tests.rs](../../crates/ferreth-compiler/src/compiler/symbolic/tests.rs) | Frontend, artifact, execution, verifier rejection and ignored capacity regressions. |
| [Research index](README.md) and this report | RF-018 evidence and Phase 2.2 status. |

The diff leaves the verifier, executor, isolated arithmetic, public
routing and the four LF-026 tests intact. Manifest and lockfile checks
found no dependency changes. The pre-existing architecture edit retained
Git blob `141d4a12172e3152716a5a92e869e05fda95c918`.

**Phase 2.2 candidate gate met.** The Phase 1 PUSH1/STOP layout passed
candidate parity, distant transfer execution and the measured capacity
gate on the tested host and backend. The shorter private STOP image and
symbolic display source are the candidate deltas for maintainer review.
LF-026 remains open on the public path. Phase 2.2 stops here for review.

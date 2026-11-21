# Phase 2.4 public cutover review

**RF-026 resolved. RF-027 closed.** This review covers the Phase 2.4 worktree
on top of [Phase 2.3d](phase-2.3d-integration-review.md) at
`0954c8bd34c60bc6cc710763291a6de1ff9c4423`. The repair changes tests, the
static accessor's Rustdoc and the Phase 2.4 research records. Production
routing, machine-code generation and dynamic instruction accounting were kept
as reviewed. ADR-001 governs the guest ABI. The frozen gate covers the
current RV64 and Cancun reference regressions. EELS and gas behavior were
outside this review.

## RF-026 — Independent cutover checks restored

The original review found that the Phase 1.6 checker skipped publisher
word and terminal JALR checks on the production side, and skipped some
publishers on the experimental side. Label inventory checks were missing
on both sides. Publisher PC metadata was compared by same-length membership,
which could accept reordered or duplicate addresses. The isolated ADD/SUB
test also compared two routes through the symbolic compiler after cutover,
leaving the retained legacy singleton linkers unused.

| Regression requirement | Implemented and verified evidence |
|---|---|
| Verify every resident publisher by semantic role and physical order | The [Phase 1.6 comparison](phase-1/tests/mixed_symbolic_guest.rs) derives the first publisher PC from exact per-op fragment sizes. It checks the entire tail, including each role's exact RV64 words and decoded terminal JALR. Experimental order is Normal, Overflow, Underflow for every vector. Public order is Normal followed by each required role in Underflow, Overflow order. Exact ordered equality binds the public PC slice to the checked role addresses. The public [final-image integration tests](../../crates/ferreth-compiler/tests/rv64_final_image.rs) independently check the literal words and ordered offsets on empty, STOP, PUSH1-only, isolated ADD/SUB and mixed images. |
| Check exhaustive, duplicate-safe labels | The Phase 1.6 checker compares the full set of expected names against all assembler labels and checks duplicate names against the raw label inventory. It asserts the expected PCs of entries, continuations, traps, arithmetic cores and return anchors, and enforces unique code sites while allowing intentional continuation aliases. Both experiments and production use their respective symbolic names, including `push1_continue_N` and `arith_update_N` on the public side. |
| Compare the full isolated ADD/SUB images to their retained legacy singleton linkers | The [orchestrator unit test](../../crates/ferreth-compiler/src/orchestrator.rs) calls `link_guest_add` and `link_guest_sub` directly. For bytecode `01` and `03` the public singleton matches all 232 legacy bytes and both legacy publisher PCs, `ENTRY_PC+172` and `ENTRY_PC+200`. The different legacy and public sources are asserted. The seeded public adapters also reproduce the public singleton artifacts. Separate integration tests exercise normal execution from stack `[2,3]` and underflow from an empty stack. |
| Preserve independent arithmetic, transfer and runtime checks | All 17 original Phase 1.6 vectors retain the decoded branch/JAL, per-occurrence leaf bytes and hashes, return, stack and ABI register, guard, staged publication and split instruction count checks. STOP-only and PUSH1-only add two vectors for zero resident exceptions and Overflow-only respectively. The existing Underflow-only and dual-role vectors complete the inventory matrix. The legacy verifier role/trampoline mutation tests and public STOP JAL and assembly round trip remain in the gate. |

The new controls first validate a known-good fixture. Public mutations swap
the two exceptional bodies while leaving their PCs unchanged, replace the
Overflow exit JALR with a jump to metadata, reorder the PC metadata, and
duplicate the Underflow PC in the Overflow slot. Each mutation triggers the
intended check. On the Phase 1.6 side, changing the unreachable experimental
Overflow body in a STOP-only image triggers the publisher-byte check. An
extra same-address label alias with identical bytes and a synthetic duplicate
name in an equal-length assembler label inventory trigger the corresponding
label checks. These seven negative controls exercise the test assertions
directly. Expected words come from literal encodings or, for singleton full
images, the retained legacy linkers.

## RF-027 — Public static exceptional count approved

The maintainer approved the number of **resident exceptional publisher
words** for `CompiledRv64Program::exceptional_termination_instructions()`.
This supersedes the cutover plan's original single-path proposal.

| Resident exceptional roles | Static words | Directed public test inputs |
|---|---:|---|
| Neither | 0 | Empty, `00` (STOP) |
| Underflow only | 8 | `50` (POP) |
| Overflow only | 8 | `6001` (PUSH1) |
| Both | 16 | `600150` (PUSH1, POP) |

The table agrees with the public count test, the independently checked role
layouts and the accessor's Rustdoc. Source inspection confirms the composer
derives the static count from the resident exceptional tail. During
exceptional execution only the selected publisher runs, for eight dynamic
instructions. Normal execution runs the seven-instruction Normal publisher.
The observed split counts, API signatures and benchmark payloads are
preserved. The retained legacy mixed test oracle still exposes its
historical static count of eight.

## LF-026 and CLI observations

The four unchanged tests
`repeated_push1_47_eof_completes_normally`,
`repeated_push1_47_stop_completes_normally`,
`repeated_push1_47_eof_full_stack_publishes_overflow`, and
`repeated_push1_47_stop_full_stack_publishes_overflow` all passed in the
focused run and workspace run. Their helpers and expected semantic assertions
are unchanged: Normal/code 0, all 47 twos, depth 47 and `s1=0x801015e0`;
or GuestException/code 1, all 1,024 seeded words unchanged, depth 1,024 and
`s1=0x80109000`. Both 46-PUSH controls also passed. Public helpers decode
the first trampoline and compare its target with Overflow metadata.

The separate passing private long-layout regression checks the first
trampoline at entry+40: +2056 for EOF and +2060 for STOP, normal counts
470+7 and 471+7, and overflow 4+8. It checks guarded memory and
instruction-by-instruction publication. These private checks complement,
but do not replace, the unchanged public outcome assertions.

`cargo run --frozen -q -p ferreth-compiler -- execute --op PUSH1 --value 5`
exited 0 and printed symbolic assembly, machine-code words, final stack `[5]`
and 10+7=17 instructions. `cargo run --frozen -q -p ferreth-compiler --
execute --bytecode 600560030100` exited 0 with Normal, stack `[8]` and 70
instructions. The bytecode CLI does not print assembly/bytes. Independent
assembly round-trip checks run in the test suite. rvemu device-tree warnings
also appeared on stdout.

## Frozen verification ledger

Run the following commands from the repository root with `CARGO_TARGET_DIR`
set to a fresh directory, as specified in the [cutover plan](phase-2.4-cutover-plan.md).
This run used Darwin arm64, Rust and Cargo 1.98.1, the frozen Cargo.lock,
asm-rs 0.2.0, rvemu 0.0.11 and REVM 43.0.3. Local raw stdout, stderr and
exit status are under ignored `target/phase-2.4-rf026-rf027.vGlOZk/`,
with the fresh build under `build/` in that directory.

| Command | Observed |
|---|---|
| `git diff --check` | Pass |
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --frozen --workspace --all-targets -- -D warnings` | Pass |
| `cargo test --frozen -p ferreth-compiler --lib` | 84 passed, 2 ignored |
| `cargo test --frozen -p ferreth-compiler --lib isolated_arithmetic_images_match_legacy_singleton_linkers` | 1 passed |
| `cargo test --frozen -p ferreth-compiler --lib public_publisher_inventory_counts_resident_exceptional_bodies` | 1 passed |
| `cargo test --frozen -p ferreth-compiler --lib standalone_arithmetic_core_words_and_fingerprints_match` | 1 passed |
| `cargo test --frozen -p ferreth-compiler --test rv64_final_image` | 7 passed |
| `cargo test --frozen -p ferreth-compiler --test rv64_push1_program` | 23 passed |
| `cargo test --frozen -p ferreth-conformance --test rc_revm_conformance` | 4 passed |
| `cargo test --frozen -p ferreth-linking-spike` | 26 passed, 1 ignored, including 8 passes in the mixed experiment |
| `cargo test --frozen --workspace -- --list` | 215 tests, 0 benchmarks across 33 target summaries |
| `cargo test --frozen --workspace --no-fail-fast` | 212 passed, 0 failed, 3 ignored, 0 measured, 0 filtered across 33 target summaries |
| `cargo test --frozen -p ferreth-compiler --lib capacity -- --ignored --nocapture --test-threads=1` | 2 passed, 84 filtered |
| `cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1` | 1 passed |
| `cargo run --frozen -q -p ferreth-conformance -- bench guest --format json` | Guest JSON matches baseline |
| `cargo run --frozen -q -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json` | Arithmetic JSON matches baseline |

### Exact inventory

Phase 2.3d discovered 205 tests (198 passed, four LF-026 failures, three
ignored). The first Phase 2.4 review discovered 206 tests (203 passed, three
ignored). Its only addition was
`compiler::symbolic::tests::public_publisher_inventory_counts_resident_exceptional_bodies`,
and the four existing LF-026 tests passed.

The final discovery diff accounts for one moved and renamed test and nine
additions. `compiler::symbolic::tests::arithmetic::isolated_arithmetic_images_match_symbolic_singletons`
became `orchestrator::tests::isolated_arithmetic_images_match_legacy_singleton_linkers`.
All other tests from the earlier inventory remain present.

| Target | Added test names |
|---|---|
| `rv64_final_image` | `verified_images_without_exception_publishers_end_after_normal`, `publisher_check_rejects_an_incorrect_terminal_jalr`, `publisher_check_rejects_duplicate_metadata`, `publisher_check_rejects_reordered_metadata`, `publisher_check_rejects_swapped_exception_bodies_with_unchanged_pcs` |
| `mixed_symbolic_guest` | `normal_only_and_overflow_only_production_inventories`, `experimental_layout_rejects_a_wrong_unreachable_publisher_body`, `label_inventory_rejects_duplicate_names`, `production_layout_rejects_an_unexpected_label_alias` |

The three opt-in tests are `pop_dup1_capacity_probe`,
`push1_capacity_boundary` and `measure_symbolic_guest_capacity`. Explicit
probe runs passed all three. The workspace reconciles as 212 passed plus
three ignored equals 215 discovered.

### Capacity observations

The compiler accepted 49,999 POPs with 100,000 labels and rejected 50,000
at the assembler label limit. It accepted 16,382 DUP1s and rejected 16,383
at the code reservation. For PUSH1 it accepted 23,829 at both EOF and STOP
and rejected 23,830 because the image overlaps the exit slot. The Phase
2.0 experimental probe checked the default 100,000-label boundary and
configured limits. Its own POP source fit 33,332 instances and hit the
label limit at 33,333. These are the same boundaries as the previous gate.

## Benchmark and arithmetic-core evidence

The complete parsed guest JSON equals the [Phase 0 guest baseline](../../runs/phase-0/guest-benchmarks.json),
including five opcode and ten program measurements. The complete parsed
arithmetic JSON equals the [approved arithmetic baseline](../../runs/rv64-arithmetic.json).
Replay by capturing command stdout, then verifying that every line before
the opening JSON object is one of the two observed rvemu device-tree
warnings (missing binary or the suggestion to install `dtc`). Compare the
entire parsed JSON object with the corresponding baseline. The guest prefix
was 30 lines and 2,445 bytes. The arithmetic prefix was four lines and 326
bytes. No JSON fields were normalized. Raw stdout SHA-256 values match the
previous review:

| Command | Raw stdout SHA-256 |
|---|---|
| Guest benchmark | `5fe319d3b352fd0e800decf469b8b17553df9c3848359a9f77f0d363a1c123af` |
| Arithmetic benchmark | `f4f44b247a0d506779e306b2956d147ba4a216d67a8293295f641660a64c9f71` |

ADD and SUB each passed all 1,012 cases. Each core had 34 static words,
34 dynamic instructions per case and 34,408 executed in total. ADD retained
FNV-1a64 `0xf1fff32ff47962f3` and SUB retained `0x9f008f320eeb7a7b`. The
focused core-identity unit test and the Phase 1.6 runtime and artifact tests
also verified embedded core bytes against the standalone cores.

## Maintainer review

Phase 2.4 meets the cutover gate with RF-026 resolved and RF-027 closed.
Legacy mixed, PUSH1 and isolated arithmetic linkers remain available as
test oracles. The worktree is ready for maintainer review before commit.

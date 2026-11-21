# Phase 2.3c — Private symbolic ADD and SUB

## Review gate

**RF-022 — Occurrence-local arithmetic calls preserve the fixed layout and guest ABI.**
The private candidate now lowers ADD and SUB through the Phase 2.3a
fragment contract. It reuses the 34-word arithmetic leaves and verifies
their exact words, control-transfer sites, and return bindings before it
constructs a compiled artifact. Directed guest runs met the private parity
gate. The milestone stops for maintainer review. Public compilation and
isolated arithmetic execution continue using the legacy routes.
[Phase 2.3c plan](phase-2.3c-plan.md) ·
[Phase 1.6 evidence](phase-1/results/1.6-mixed-symbolic-guest.md) ·
[ADR-001](../../docs/adr/001-evm-guest-state-abi.md)

## Starting revision and scope

`git status --short --branch` returned only
`## feat/symbolic-guest-composer`. `git rev-parse HEAD` returned
`82b14977318bb0538334210e3bce8d7fd3db4047` and `git diff --stat` was
empty. That revision records the maintainer's acceptance of Phase 2.3b.1,
including closure of RF-019 and the known RF-020 implementation limit.
The checked-in Phase 2.3c plan still carried its proposed status. The
maintainer's task explicitly approved implementation under that plan.
Proposed ADR-002 supplied gas-metering context. This phase follows ADR-001.
[Phase 2.3b.1 acceptance](phase-2.3b.1-boundary-hardening.md)

## Fragment generation and independent verification

The private frontend selects the ADD or SUB emitter and names the next
fragment, STOP, or Normal at EOF. The arithmetic emitters build complete
fragments with source, an entry, local transfers, publisher declarations,
and a fixed-layout verification contract. The shared wrapper uses four
symbols per occurrence, keyed by the decoded instruction ordinal `n`.

| Symbol | Word | Purpose |
|---|---:|---|
| `guest_arithmetic_n` | 0 | Entry. Compute `s0 + 64` and branch on unsigned `s1 < s0 + 64` before accessing operands. |
| `arith_update_n` | 6 | Resume after `jal ra` and decrement `s1` by 32. |
| `arith_underflow_n` | 8 | Jump to the Underflow publisher with code 2. |
| `arith_core_n` | 9 | Enter the leaf with `a0 = s1 - 32`, `a1 = a2 = s1 - 64`. |

Word 7 names the next entry or publisher directly. The leaf computes
in RV64, stores in the lower operand slot, and returns at word 42 via
`jalr x0, 0(ra)`. SUB computes the top word minus the next word, modulo
2²⁵⁶. The standalone and symbolic generators write the same instruction
body. Public standalone assembly keeps its original directives and labels.

The core `compose(&[SymbolicFragment])` and its verification dispatch are
unchanged. The composer places fragment source, derives the publisher
inventory, assembles the image once with `asm-rs 0.2.0`, resolves labels,
passes declarations and fixed-layout descriptors to the verifier, and
returns the existing artifact. Opcode selection stays in frontend lowering.

The declaration scan independently decodes the branch and JAL words,
checks their actual register fields and signed targets, and rejects extras,
omissions and duplicates. For the exact leaf return word `00008067`, it
reads the machine word preceding the declared update anchor, verifies a
`jal ra`, and checks that the decoded call destination starts the 34-word
leaf ending at that return. The anchor must hold `fe048493`
(`addi s1, s1, -32`). The fixed-layout pass derives offsets from the
sequential fragment layout. It requires the preflight and pointer words,
the depth branch at word 1, the call at word 5 targeting this occurrence's
word 9, the update at 6, continuation to the next fragment at 7, the
Underflow transfer at 8, and every word of the selected core through the
return at 42. The canonical call word is `010000ef`.
The existing alignment, executable-bound and exact publisher-body checks
also apply. A relaxation or expansion that changes this layout fails
verification.
[Symbolic wrapper](../../crates/ferreth-compiler/src/rv64/opcodes/arithmetic.rs) ·
[Final-image verifier](../../crates/ferreth-compiler/src/rv64/verify.rs)

## Exact arithmetic core identity

The baseline was rebuilt from `git archive 82b14977318bb0538334210e3bce8d7fd3db4047`
with the frozen lockfile. For each opcode, assemble its baseline
`Opcode::assembly()` after filtering ELF directive lines, then read each
four-byte chunk as a little-endian `u32`. The verifier compares each
embedded word at fragment words 9 through 42 with that baseline. The
test also compares the 136-byte candidate and legacy leaf slices directly
with the current standalone bytes, including repeated occurrences.
There were zero word differences. The following is the baseline word
sequence in address order.

```text
ADD
00000e13
00053283 0005b303 006283b3 0053beb3 01c383b3 01c3bf33 01eeee33 00763023
00853283 0085b303 006283b3 0053beb3 01c383b3 01c3bf33 01eeee33 00763423
01053283 0105b303 006283b3 0053beb3 01c383b3 01c3bf33 01eeee33 00763823
01853283 0185b303 006283b3 0053beb3 01c383b3 01c3bf33 01eeee33 00763c23
00008067

SUB
00000e13
00053283 0005b303 406283b3 0062beb3 41c382b3 0053bf33 01eeee33 00563023
00853283 0085b303 406283b3 0062beb3 41c382b3 0053bf33 01eeee33 00563423
01053283 0105b303 406283b3 0062beb3 41c382b3 0053bf33 01eeee33 00563823
01853283 0185b303 406283b3 0062beb3 41c382b3 0053bf33 01eeee33 00563c23
00008067
```

FNV-1a uses initial value `0xcbf29ce484222325`. For each machine-code byte
in address order, XOR the byte into the hash and multiply by
`0x100000001b3` modulo 2⁶⁴. The 136 bytes include the leaf return and
exclude the wrapper, publishers and sentinel. The observed fingerprints
are ADD `0xf1fff32ff47962f3` and SUB `0x9f008f320eeb7a7b`.
[Core test](../../crates/ferreth-compiler/src/compiler/symbolic/tests/arithmetic.rs) ·
[Approved benchmark baseline](../../runs/rv64-arithmetic.json)

## Runtime and differential evidence

The runtime tests replay all 17 Phase 1.6 specimens and add 20 directed
specimens, including the explicit STOP case with unreachable arithmetic.
Each route uses identical bytecode, seeded stack, initialized registers
and ABI memory. The tests assert independently calculated U256 results
for both routes, the full live stack, status, exception code, live and
published `s1`, and preservation of `s0`, `sp`, `gp`, `tp`,
`s2`–`s11`, the two stack guards and the native stack guard. During
successful arithmetic each call writes its own update PC to `ra`. The
leaf reaches that PC in 34 executed instructions while preserving `s1`,
then the update retires one word. At depths zero and one the executor
takes a three-instruction path with no operand loads or stores, preserves
stack bytes and `ra`, and publishes Underflow. A successful ADD followed
by an underflowing SUB retains the ADD result and the last successful
call's `ra`.

The tests sample the metadata record at every publisher instruction and
check the store words and the order final pointer, exception code, status.
The final JALR reaches `0x800ffffc`. Placing EBREAK at the sentinel exposes
an accidental fetch, and the completed runs exclude it from the count.
Representative observed results are below. Full vectors and per-route
PCs and byte ranges can be replayed with the focused tests and `--nocapture`.

| Bytecode or case | Initial stack, bottom to top | Result | Body + publisher |
|---|---|---|---:|
| `6002600301` | Empty | Normal, [5] | 62 + 7 |
| `600260030300` | Empty | Normal, [1] | 63 + 7 |
| `60028001` / `60028003` | Empty | Normal, [4] / [0] | 66 + 7 |
| `01` / `03` | Empty or one full-width word | Underflow (2), initial stack preserved | 3 + 8 |
| `03` | [3, 2] | Normal, [2²⁵⁶−1] | 42 + 7 |
| `01` | [2²⁵⁶−1, 1] | Normal, [0] | 42 + 7 |
| `01` / `03` | [2¹⁹²−1, 1] / [1, 2¹⁹²] | Normal, [2¹⁹²] / [2¹⁹²−1] | 42 + 7 |
| `01` / `03` | Seed-42 full stack, 1,024 words | Normal, 1,023 words with exact low-slot result | 42 + 7 |
| `0101` / `0303` / `0103` | [2, 3, 6] | Normal, [11] / [1] / [7] | 84 + 7 |
| `600260030180600703506009018050600803` | Empty | Normal, [2²⁵⁶−6] | 252 + 7 |
| Same mixed input + STOP + unreachable `0103fe60` | Empty | Normal, [2²⁵⁶−6] | 253 + 7 |

Both routes reassemble their displayed assembly to the executed image.
At Normal PC `P`, legacy places Overflow and Underflow at `P+28` and
`P+60`. The candidate emits Underflow at `P+28` and emits Overflow at
`P+60` only when PUSH1 or DUP1 declares it. The artifact checks compare
every body word and find differences exactly at exceptional trampoline
JALs. Publisher bodies match by role. Arithmetic-only and POP-only
images save 32 bytes in the candidate. Programs with both exceptional
roles have equal lengths. For example, ADD alone emits 232 candidate
bytes and 264 legacy bytes with Normal at `0x800000ac`. The repeated
mixed EOF specimen emits 1,160 bytes on each route. Its Normal,
Underflow and Overflow PCs are `0x8000042c`, `0x80000448` and
`0x80000468` in the candidate. Legacy exchanges the two exceptional
roles at those PCs. The changed body byte offsets, in hexadecimal, are
`28 54 78 13c 140 16c 190 228 254 278 33c 340 350 37c 3a0`. The artifact field
`exceptional_termination_instructions()` reports all exceptional
publisher words for the candidate and only the eight-word Overflow body
for legacy. Executed exceptional publishers take eight instructions on
both paths. No runtime-count differences were observed.

## Negative verifier evidence

| Mutation | Rejection |
|---|---|
| Call to another occurrence's core, wrong link register, corrupted displacement | Decoded target, destination register or target alignment |
| Return anchor moved to another update or stack decrement altered | Call/return binding or exact update word |
| Leaf return with wrong register or immediate | Exact leaf return instruction |
| Continuation to a leaf and Underflow to Normal or Overflow | Decoded transfer target |
| Missing or duplicate call declaration, unexpected core JAL | Declaration coverage |
| Changed core word, pointer setup word, or expanded depth branch | Exact arithmetic layout or decoded branch check |
| Call and return removed together with their declarations | Required fixed-layout call at `0x80000014` |
| Both calls swapped, with return and call declarations adjusted to match | Required occurrence-local target at `0x80000014` |
| Underflow word and declaration both changed to Overflow | Required Underflow target at `0x80000020` |
| Repeated ADD or mixed ADD/SUB with reused ordinal, and collision of only the leaf label | Composer entry uniqueness or assembler label uniqueness |

The machine-word and declaration mutations assembled successfully and
reached final-image verification. Entry collisions are caught before
assembly and duplicate leaf definitions are rejected by the assembler.
The agreeing-declaration controls exercise the independent fixed-layout
checks.
[Negative fixtures](../../crates/ferreth-compiler/src/compiler/symbolic/tests/arithmetic.rs)

## Resource policy

The default `asm-rs 0.2.0` resource limits and ADR-001 executable-image
bound still apply independently. The wrapper adds only the four symbols
listed above. The existing opt-in probes exercise controlled label,
relocation and image-bound failures under the accepted capacity policy.
[RF-019 and RF-020](phase-2.3b.1-boundary-hardening.md)

## Verification

Commands ran on arm64 macOS with Cargo and rustc 1.98.1, the frozen
lockfile, `asm-rs 0.2.0` and `rvemu 0.0.11`. Run these from the workspace
root. The focused arithmetic suite can be replayed with `-- --nocapture`
to print per-vector publisher PCs, changed word offsets and metadata traces.

| Command | Result |
|---|---|
| `cargo test --frozen -p ferreth-compiler --lib compiler::symbolic::tests::arithmetic` | 6 passed, 0 failed, 0 ignored, 73 filtered. Also passed with `-- --nocapture`. |
| `cargo test --frozen -p ferreth-compiler --lib compiler::symbolic::tests` | 18 passed, 0 failed, 2 ignored, 59 filtered. Includes empty/STOP-only, PUSH1 inventory, 46/47 PUSH1 normal and overflow, and POP/DUP1 boundary regressions. |
| `cargo test --frozen -p ferreth-compiler --lib rv64::verify::tests` | 13 passed, 0 failed, 0 ignored, 66 filtered. |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --frozen --workspace --all-targets -- -D warnings` | Passed. |
| `cargo test --frozen -p ferreth-linking-spike` | 22 passed, 0 failed, 1 ignored. |
| `cargo test --frozen --workspace -- --list` | 199 tests, 0 benchmarks. Six new tests relative to Phase 2.3b.1. |
| `cargo test --frozen --workspace --no-fail-fast` | 192 passed, 4 failed, 3 ignored, 0 measured, 0 filtered. Exit 101. |
| `cargo test --frozen -p ferreth-compiler --lib pop_dup1_capacity_probe -- --ignored --nocapture --test-threads=1` | 1 passed, 78 filtered. Retained 49,999/50,000 POP label boundary and 16,382/16,383 DUP1 image boundary checks. |
| `cargo test --frozen -p ferreth-compiler --lib push1_capacity_boundary -- --ignored --nocapture --test-threads=1` | 1 passed, 78 filtered. Retained 23,829/23,830 PUSH1 image boundary at EOF and STOP. |
| `cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1` | 1 passed. Retained the 100,000-label limit and spike image-bound checks. |
| `cargo run --frozen -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json` | Exit 0. Both cores passed 1,012/1,012 cases at 34 words and 34 executed instructions per case, 34,408 per core. |

The workspace ran 196 tests. Its only failures were the four pre-existing
LF-026 controls in `rv64_push1_program` that exercise the legacy public
route. Their names and expectations are unchanged. The opt-in runs are
separate from the workspace totals.

| LF-026 test | Rejected legacy displacement |
|---|---:|
| `repeated_push1_47_eof_completes_normally` | +2,056 |
| `repeated_push1_47_eof_full_stack_publishes_overflow` | +2,056 |
| `repeated_push1_47_stop_completes_normally` | +2,060 |
| `repeated_push1_47_stop_full_stack_publishes_overflow` | +2,060 |

Each failure was an `ImmediateOverflow` at line 12, column 5 during
legacy display-assembly reassembly, with numeric bounds −2,048 through
+2,047. In the benchmark capture, four rvemu device-tree warning lines
(326 bytes) preceded the JSON on stdout. After checking those lines,
the parsed JSON matched the approved baseline in full. The approved
artifact was left in place.
[Approved benchmark baseline](../../runs/rv64-arithmetic.json)

The first focused run exposed fixture assumptions. The legacy static
exceptional metric is eight words, the pinned assembler emits a
two-byte `.word` and a four-byte `.long`, and the missing-call test
expected a different diagnostic phrase. The fixtures now assert the
legacy metric as emitted, inject the corrupted four-byte call with
`.long`, and check the fixed-layout call diagnostic. The focused
rerun passed all six tests.

## Changed files

| File | Change |
|---|---|
| `crates/ferreth-compiler/src/compiler/symbolic.rs` | Private frontend lowering and continuation names. |
| `crates/ferreth-compiler/src/compiler/symbolic/tests.rs` | Register the arithmetic tests and remove the superseded private ADD rejection expectation. |
| `crates/ferreth-compiler/src/compiler/symbolic/tests/arithmetic.rs` | Baseline core, runtime parity and final-image mutation regressions. |
| `crates/ferreth-compiler/src/compiler/symbolic/tests/pop_dup1.rs` | Reuse the runtime observer with arithmetic checkpoints and additional register and guard assertions. |
| `crates/ferreth-compiler/src/lib.rs` | Share the established leaf instruction writers with standalone generation. |
| `crates/ferreth-compiler/src/rv64/opcodes/add.rs` | Private ADD emitter. |
| `crates/ferreth-compiler/src/rv64/opcodes/arithmetic.rs` | Shared symbolic wrapper and declarations. |
| `crates/ferreth-compiler/src/rv64/opcodes/mod.rs` | Register the private arithmetic module. |
| `crates/ferreth-compiler/src/rv64/opcodes/sub.rs` | Private SUB emitter. |
| `crates/ferreth-compiler/src/rv64/verify.rs` | Check local return bindings and independently verify arithmetic wrapper and baseline leaf words. |
| `research/guest-linking/phase-2.3c-symbolic-arithmetic.md` | Phase evidence, regression traceability and review gate. |
| `research/guest-linking/README.md` | Research index. |
| `docs/research/ferreth-linking-architecture.md` | Phase 2 roadmap. |

## Phase 2.3d handoff

The runtime findings cover directed programs under rvemu 0.0.11.
The standalone benchmark uses REVM 43.0.3 for Cancun arithmetic parity.
The private candidate addresses the long PUSH1 controls from earlier
phases. LF-026 still affects the public legacy path. Maintainer review
must precede any routing change. A future cutover must verify each public
entry point and its artifact and metric contracts, including the
exceptional publisher inventory and metadata field identified above.

# ADR-002 implementation plan

Maintainer approval recorded on 2026-11-23. Phases 1 and 2 implemented on the experimental branch. Read the [accepted ADR](../adr/002-cancun-evm-gas-metering.md), [baseline assessment](adr-002-readiness.md), then [raw corpus and fixture table](adr-002-reference-corpus.md). This plan proposes test locations and future commits. Active portable fixtures are V01–V07 and V32–V33. Phase 2 is ready for human review. Phases 3–5 remain pending.

## Test boundaries and common gate

- **A — portable frame results.** Add crates/ferreth-conformance/tests/adr002_frame_gas.rs with corpus IDs retained in failure output. Use the recorded class, settled gas, gross used and successful stack as independent expected data. Also run the pinned direct interpreter with identical inputs and preserve raw diagnostics, applying the documented exceptional normalization only to the reference. Compare Ferreth's actual guest publication directly. Keep transaction comparisons in rc_revm_conformance.rs.
- **B — Ferreth diagnostics and effects.** Add crates/ferreth-compiler/tests/rv64_frame_gas.rs for explicit-budget executions. Assert exact guest codes, live stack words, depth and pointer, including effects from successful prefix operations. Use existing stack seeding and guard-byte patterns in rv64_dup1.rs for physical memory checks.
- **C — ABI and image integrity.** Extend compiler integration tests rv64_guest_initialization.rs, rv64_guest_result.rs, rv64_executor.rs and rv64_final_image.rs. Extend compiler/symbolic/tests.rs near observe_publication and the misdirected-continuation controls. Inspect guest memory at selected instruction boundaries and independently verify final machine words.

For each implementation phase, first introduce only the fixtures for that increment, run a named test and retain the actual assertion failure. If an API addition initially prevents compilation, migrate callers to the canonical signature on the common execution route. Rerun the fixture to observe the missing publication, incorrect gas or incorrect termination. A compiler error alone does not demonstrate RED. Implement GREEN, then rerun the focused and common gates. Commit each completed increment with compiling, passing tests. Avoid ignored future cases. Record active IDs at each stop.

Common regression gate for phases 1–5, in addition to the commands specified below:

```sh
cargo fmt --all -- --check
cargo clippy --frozen --workspace --all-targets -- -D warnings
cargo test --frozen --workspace --no-fail-fast
```

Develop phases 1–5 on an experimental implementation branch. Add the budget parameter once to the common execution route. All callers execute the same current opcode bodies, including those still awaiting charges. Intermediate gas results are development observations with verified coverage limited to the active fixtures. Continue running the entire existing semantic regression suite at every increment. Release the public gas contract only after the full milestone gate.

Use ordinary Rust visibility for the canonical bytecode entry on the unreleased branch so the separate conformance crate can call it from Phase 1. Keep seeded-stack and instruction-stepping hooks internal where practical. This uses the final public signature immediately and avoids a test-only execution interface or feature flag. In each of phases 1–4, add that phase's Layer A rows before changing guest behavior. Run `cargo test --frozen -p ferreth-conformance --test adr002_frame_gas` against pinned direct REVM and recorded expectations at RED and again at GREEN, retaining every earlier row. A phase can contain already-passing boundary cases alongside a failing behavioral witness.

Existing byte/count fixtures encode the old unmetered image. At each intentional layout change retain semantic comparisons, protected leaf bytes and corruption tests, and give the new image independently derived expectations for publisher order, words and transfers. Retain historical fixtures with their original scope. Do not mechanically bless new output.

## Phase 0 — human decision

Inputs are all V01–V37, the two runners, ADR-001 and the research record. Review the 37 distinct rows, 19 successes, 18 exceptions and V22's observed 14 versus derived 0. Check links and use git diff --check. RED, GREEN, refactoring and executable test additions do not apply to this documentation phase. The maintainer completed this approval gate on 2026-11-23. Dependency is completed reference research. Proposed commit boundary is the reviewed documentation alone.

## Phase 1 — budget round trip

Depends on Phase 0. Deliver branch-local API availability, initialization and terminal publication as one green commit. V01/V03 must succeed at zero, V02 at 7 with unreachable GAS, all with empty stacks and zero used. Additionally run both 00 and empty bytecode at budgets 1, 7, 2^63 and u64::MAX. For every case require Normal, code 0, initial limit equal to input, remaining equal to input, used 0, empty stack and s1 equal to s0. Reuse existing private stack setup.

1. In orchestrator.rs change the public signature in place to `execute_guest_evm_bytecode(bytecode: &[u8], gas_limit: u64)`. Migrate all workspace callers. Resolve an omitted CLI budget to `u64::MAX` at argument handling and test omission, explicit zero and a positive budget. Pass the resolved value through the common execution implementation to the initializer in rv64/guest.rs. Isolated APIs use this same implementation with their initial stack. In rv64_frame_gas.rs propose stop_and_eof_publish_the_entire_budget. Run V01–V03 through the conformance crate immediately. With the terminal gas slot initialized to zero and the old Normal publisher, V02 must fail validation or the expected remaining-gas assertion (7). V01 and V03 can already pass at zero. Retain the publisher-store witness below for those cases.
2. In rv64_guest_initialization.rs add initializes_frame_budget_before_execution. Reinitialize a previously populated executor and inspect six aligned words at metadata base. Expect [0, 0, stack_base, limit, limit, 0], s0=s1=stack_base and preserved native sp. In GuestMetadata::encode/decode constrain the old final-s1 slice to bytes 16..24 when extending the length to 48. Verify little-endian field boundaries and exact length.
3. In symbolic/tests.rs add normal_publisher_writes_remaining_before_status. For budget 7, inspect RUNNING and result slot 0 at publisher entry. In the test harness overwrite the result slot with a distinct marker, then single-step or stop at instruction boundaries using the existing observation helper. Require result 7 while status is still RUNNING, then NORMAL, then sentinel arrival before fetch. Repeat at budget zero with a nonzero marker. This catches a missing store even when the initialized result would accidentally match. Reading the same initialized input field in Rust cannot pass this test.
4. For GREEN, extend compiler/publisher.rs::Publisher::assembly. Load live gas from +32 and store it at +40 on normal completion. Bring existing exceptional publishers onto the same ABI by writing zero at +32 and +40. Write status last on every path. Extend rv64/verify.rs::Publisher::verify and publisher layout/length expectations in compiler/symbolic.rs::verified_program. In rv64_final_image.rs require rejection of an omitted or moved gas store and premature status publication. Keep independent body and publisher checks active.
5. In guest.rs::read_guest_result verify the sentinel execution boundary through the existing execution flow, original limit, terminal status/code, pointers, live/published equality and remaining <= limit. In rv64_guest_result.rs add rejects_inconsistent_frame_gas for limit mismatch, remaining above limit, an exceptional nonzero result and RUNNING metadata. Keep instruction-limit and injected backend faults on the existing host error path. A fault before terminal publication cannot yield a valid result.
6. Run cargo test --frozen -p ferreth-compiler --test rv64_frame_gas and cargo test --frozen -p ferreth-compiler --test rv64_final_image plus the common gate. Exit when both reachable STOP and empty EOF round-trip all budgets through guest stores, the mutation controls fail as intended, and existing callers pass. Refactoring is limited to shared budget threading and metadata encoding.

### Phase 1 execution evidence (2026-11-23)

Working tree on `feat/adr-002-gas-metering` at base `0e0a9f7`, including the uncommitted explicit-budget API migration. The initial behavioral command was `cargo test --frozen -p ferreth-conformance --test adr002_frame_gas`. V01 and V03 passed. V02 produced the following diagnostics with the original guest publisher and compiling zero-valued result accessors.

```text
V02 REVM Stop, remaining=7, used=0, stack=[], pc=1
assertion `left == right` failed: V02 published remaining gas
  left: 0
 right: 7
```

The initialization RED used `cargo test --frozen -p ferreth-compiler --test rv64_guest_initialization initializes_frame_budget_before_execution`. All three gas words retained the prefilled value `u64::MAX`. The expected words were `[0x0123456789abcdef, 0x0123456789abcdef, 0]`. With the extended initializer and original publisher, `cargo test --frozen -p ferreth-compiler --lib normal_publisher_writes_remaining_before_status` failed with `missing guest publication at budget 7`. With the new publisher, the same test observed the marker through instruction 5, the actual budget with RUNNING after instruction 6, NORMAL after instruction 8 and the pre-fetch sentinel after instruction 9. Both 7 and 0 passed.

V01–V03 now pass against pinned direct REVM Cancun and the recorded expectations. STOP and empty EOF round-trip 0, 1, 7, 2^63 and u64::MAX. The CLI test runs the ordinary binary with omitted, zero and positive budgets. Host rejection cases include a forged input limit, disagreeing gas slots, remaining above limit, nonzero exceptional gas, invalid status/code, invalid stack pointers and an intermediate executor boundary. Omitted gas stores and status moved ahead of gas publication fail independent final-image verification.

The layout derivation starts at the end of the existing opcode body. Normal adds one LD and one SD (28 → 36 bytes). Either exceptional publisher adds two SDs (32 → 40 bytes). Every execution runs one publisher, so termination and total counts increase by two instructions. With all three publishers, entries relative to body end are 0, 36 and 76, and the tail occupies 116 bytes. Each external JAL uses target PC minus instruction PC. For the 47-PUSH1 EOF witness this gives 44 × 47 + 36 − 40 = 2064 bytes. Arithmetic body sizes, leaf hashes and the historical research image retain their original coverage. The mixed research comparison checks equal executed body counts and a two-word production publisher increment.

The capacity formulas for resident publisher tails are 44n + 76 (+4 for STOP) for nonempty PUSH1, 16n + 76 for POP, and 64n + 116 for DUP1. The POP image-only maximum decreases by one to 65,531, while the assembler label ceiling still limits executable POP programs to 49,999. The existing 23,829/23,830 PUSH1 and 16,382/16,383 DUP1 image boundary pairs still straddle the reservation.

The common gate and both opt-in capacity commands below passed. The branch-local contract covers the Phase 1 fixtures. Positive-cost charging and the remaining portable vectors await later increments.

## Phase 2 — PUSH1

Depends on Phase 1. Activate V04–V07 and V32–V33, retaining V01–V03. In rv64_frame_gas.rs add push1_charges_before_capacity_check. Check V04 OOG with unchanged empty stack, V05 [2]/0, V06 [2]/1 and V07 [0]/0 (stack/remaining). At depth 1,024 V32 must publish code 3 and V33 code 1, preserving all words, the guard and final pointer. Add positive-budget overflow leaving residual live gas before settlement, then verify zero terminal gas. At RED, V04 can execute an unfunded push. At GREEN also execute PUSH1 2 through the isolated PUSH1 API and the canonical bytecode API at u64::MAX. Require identical image bytes, stack [2], remaining u64::MAX-3 and gross used 3. In rv64/opcodes/push.rs::generate_symbolic_push1 emit the shared inline unsigned charge before capacity preflight. Add OOG to publisher roles, inventory, verifier and exceptional publisher. Include an early OOG transfer in a long program where the publisher is more than 4 KiB away. Require correct target on execution and rejection of a diverted target. Refactor only common charge emission and declarations. Run cargo test --frozen -p ferreth-compiler --test rv64_push1_program and the common gate. Exit with nine active IDs and green boundary/long-image cases. Proposed commit covers PUSH1 and its OOG path.

### Phase 2 execution evidence (2026-11-23)

The baseline was clean on `feat/adr-002-gas-metering` at `1fc5b885d50e653255ea609606b5d253c08af68a`. Verification used Rust and Cargo 1.98.1.

Before changing production code, `cargo test --frozen -p ferreth-conformance --test adr002_frame_gas -- --show-output` compiled and returned 5 passed, 4 failed. Pinned direct REVM matched each raw result, remaining value and stack. Ferreth published the following outcomes.

| ID | Ferreth RED | Assertion |
|---|---|---|
| V01–V03 | Normal, remaining 0/7/0, used 0 | Pass |
| V04 | Normal, code 0, remaining 2, used 0, depth 1 | Expected exceptional |
| V05 | Normal, remaining 3, used 0 | Expected remaining 0 |
| V06 | Normal, remaining 4, used 0 | Expected remaining 1 |
| V07 | Normal, remaining 3, used 0 | Expected remaining 0 |
| V32 | Exceptional, code 1, remaining 0, used 3074, depth 1024 | Portable pass |
| V33 | Exceptional, code 1, remaining 0, used 3075, depth 1024 | Portable pass |

Before changing production code, `cargo test --frozen -p ferreth-compiler --test rv64_frame_gas` compiled and returned 2 passed, 3 failed. Budget 0 returned Normal instead of exceptional. Three pushes with budget 10 left 10 instead of 1. A full stack with budget 3074 published code 1 instead of 3, exposing the precedence defect that portable V32 cannot distinguish.

The shared emitter in [gas.rs](../../crates/ferreth-compiler/src/rv64/opcodes/gas.rs) uses two ADDIs to derive metadata from s0. It loads live gas from +32, materializes the cost, branches unsigned to the occurrence-local debit on sufficiency, and otherwise takes a symbolic JAL to the shared OOG publisher. The debit subtracts and stores once before the original PUSH1 capacity guard. The emitter uses t0–t2. The stack, s0, s1, native sp and ra are preserved during charging. Costs admitted by the emitter fit a positive signed 12-bit immediate. Ferreth continues to compile bytecode alone.

The OOG publisher stores final s1, code 3, zero live and published gas, and exceptional status in that order, then jumps to the existing sentinel. Full-stack stepping after an earlier successful push observes 2 live gas at OOG entry and 5 live gas at overflow entry. In both cases live gas becomes zero at publisher instruction 6 while status is RUNNING. Exceptional status appears after instruction 9. Whole-stack and guard snapshots across the failing instruction, and terminal stack and pointer assertions, preserve all initial words and the successful prefix. An exact-budget push, a three-push program, a later OOG, truncated PUSH1 and the unsigned budgets 2^63 and u64::MAX pass. The canonical and isolated entry points produced identical assembly and machine code. Every tested budget for `6002` used that image.

The GREEN conformance run with `-- --show-output` prints the raw REVM result, remaining gas, gross used, stack and PC. The table records the rerun. F is 1,024 ones. The exceptional reference rows already had raw remaining zero. Ferreth's guest publication was compared directly.

| ID | Raw pinned REVM result, gas left/used, stack, PC | Ferreth status, code, gas left/used, stack |
|---|---|---|
| V01 | Stop, 0/0, [], 1 | Normal, 0, 0/0, [] |
| V02 | Stop, 7/0, [], 1 | Normal, 0, 7/0, [] |
| V03 | Stop, 0/0, [], 1 | Normal, 0, 0/0, [] |
| V04 | OutOfGas, 0/2, [], 1 | Exceptional, 3, 0/2, [] |
| V05 | Stop, 0/3, [2], 3 | Normal, 0, 0/3, [2] |
| V06 | Stop, 1/3, [2], 3 | Normal, 0, 1/3, [2] |
| V07 | Stop, 0/3, [0], 3 | Normal, 0, 0/3, [0] |
| V32 | OutOfGas, 0/3074, F, 2049 | Exceptional, 3, 0/3074, F |
| V33 | StackOverflow, 0/3075, F, 2049 | Exceptional, 1, 0/3075, F |

The fixed-layout verifier checks charge words 0–7 independently, including the unsigned branch at word 4 to word 6 and OOG JAL at word 5, followed by capacity, continuation and overflow transfers at words 10, 17 and 18. It validates the OOG publisher's placement, complete words and sentinel return. Negative controls reject a signed branch, missing OOG transfer, wrong debit or live store, omitted OOG gas store, premature status, and an OOG JAL diverted to Overflow both with the original declaration and with a matching corrupted declaration. A 55-PUSH1 test takes the first OOG JAL across 4,236 bytes. Historical 46/47 witnesses retain both EOF and STOP cases and the legacy parser rejection at 47. Mixed replays preserve standalone arithmetic leaf bytes and compare the historical guest outcomes against the metered execution.

The assembled charge adds eight static words (32 bytes) per PUSH1. Seven execute on sufficiency, giving 17 body instructions for a successful push and 11 for a funded overflow. Six execute on insufficiency, followed by the ten-word, 40-byte OOG publisher, for 16 dynamic RV64 instructions on a first-instruction OOG. An isolated normal PUSH1 executed 26 instructions, including the nine-word normal publisher. The nonempty PUSH1-only image is 76n + 116 bytes at EOF and four bytes larger with STOP, up from 44n + 76 at EOF. A single PUSH1 image grows from 120 to 192 bytes, and 47 PUSH1s at EOF grow from 2,144 to 3,688 bytes. Mixed images with all four publishers have entries at body-end offsets 0, 36, 76 and 116 and a 156-byte tail. STOP and empty EOF alone retain their nine-word normal tail.

| Production PUSH1 count | EOF bytes | STOP bytes | Observation |
|---:|---:|---:|---|
| 13,795 | 1,048,536 | 1,048,540 | Layout accepted, first OOG executed for both images |
| 13,796 | 1,048,612 | 1,048,616 | Assembled, image overlaps exit slot |
| 13,797 | — | — | First OOG JAL exceeds assembler reach at 1,048,628 / 1,048,632 bytes |

The 1,048,572-byte code reservation limits production to 13,795 PUSH1s with either ending. The Phase 1 limit was 23,829. The metered layout uses four labels per PUSH1 plus three publisher labels and one more for explicit STOP. At the accepted boundary the totals are 55,183 and 55,184, below the 100,000-label limit. The production POP 49,999/50,000 label and DUP1 16,382/16,383 image pairs still straddle their respective limits. The historical research probe preserves its original unmetered layouts and independently exercises assembler limits.

The focused GREEN commands and results were as follows.

| Command | Result |
|---|---|
| `cargo test --frozen -p ferreth-conformance --test adr002_frame_gas` | 9 passed |
| `cargo test --frozen -p ferreth-conformance --test adr002_frame_gas -- --show-output` | 9 passed, raw rerun printed |
| `cargo test --frozen -p ferreth-compiler --test rv64_frame_gas` | 5 passed |
| `cargo test --frozen -p ferreth-compiler --test rv64_push1_program` | 23 passed |
| `cargo test --frozen -p ferreth-compiler --test rv64_final_image` | 9 passed |
| `cargo test --frozen -p ferreth-compiler --lib capacity -- --ignored --nocapture --test-threads=1` | 2 passed |
| `cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1` | 1 passed |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --frozen --workspace --all-targets -- -D warnings` | Passed |
| `cargo test --frozen --workspace --no-fail-fast` | 238 passed, zero failed, three opt-in probes ignored |
| `git diff --check` | Passed |

The first workspace runs after implementing the charge exposed stale Phase 1 image widths, publisher inventories and instruction-count expectations, including in the benchmark and mixed research comparison. The revised tests derive those expectations from the fixed fragment widths and executed paths. This increment covers PUSH1 charging, STOP/EOF and the shared exceptional publication path. POP, DUP1, ADD, SUB and reachable GAS remain future work. The next gate is human review of the Phase 2 diff.

## Phase 3 — existing stack and arithmetic operations

Depends on Phase 2. Activate V08–V25 and V36. Add stack_operations_charge_before_effects in rv64_frame_gas.rs. At RED, V09 exposes a missing POP charge and V17 a missing ADD charge on the same compiled route. In particular V08 must retain [2], V16/V23 retain [2,3], V19/V20 publish OOG and V21 underflow. V15 uses Ferreth underflow code 2. V22 retains [2], code 2, remaining 0 and used 20. V36 must preserve the full stack and publish overflow. Require every successful stack and gas pair listed in the corpus. In rv64/opcodes/pop.rs and dup.rs apply the established emitter at fragment entry. In arithmetic.rs::generate_symbolic_arithmetic apply it before depth validation and the local call. Adjust verifier sites and prove arithmetic leaf words still match. Refactor only shared wrapper logic exposed by these cases. Run cargo test --frozen -p ferreth-compiler --test rv64_dup1 and the common gate. Exit with 28 active IDs. Proposed green commits may split POP/DUP1 and ADD/SUB with their respective cases.

## Phase 4 — GAS

Depends on Phase 3. Activate V26–V31, V34–V35 and V37. Add gas_pushes_post_charge_u64_as_u256. RED should observe unsupported opcode 0x5a from strict decoding. Adopt GAS in lib.rs::decode_detailed and its projection, handle exhaustive consumers including legacy ProgramRunner (explicit guest-only rejection), and extend symbolic::compile dispatch and successor naming. Generate a fragment with cost 2, capacity preflight and four-limb store. Update independent layout verification. Check exact raw stack limbs at maximum budget (low u64::MAX-2, three zero limbs), and V31 across bit 63. V26/V27 require OOG without push, V28 [0]/0, V29 [8,6]/6, V34 OOG and V35 overflow, V37 [3,4]/4. Refactor common push stores only where the tests cover both forms. Run cargo test --frozen -p ferreth-compiler --test rv64_frame_gas and the common gate. Exit with all 37 IDs active and supported entry points using the complete gas path. Proposed commit covers GAS and its fixtures. Use V30 to compare the CLI with an omitted budget against the canonical API called with u64::MAX. Both must push u64::MAX-2.

## Phase 5 — full milestone validation

Depends on Phase 4. In adr002_frame_gas.rs add every_corpus_id_runs_once, validating the exact set V01–V37 and rejecting duplicates. Demonstrate RED on a locally omitted or repeated row, restore the complete inventory for GREEN. Require 37/37 class, settled and gross matches and 19/19 successful stacks. Exercise corruption of OOG targets, publisher order, continuations and leaf returns in the existing verifier fixtures. Confirm all previous capacity probes still check accepted and rejected boundaries against independently calculated metered layouts. A measured capacity change needs a documented cause. Preserve the 47-PUSH success and overflow witnesses and split dynamic instruction accounting. Refactoring is limited to duplicated fixture setup. Proposed commit covers corpus inventory, remaining integrity regressions and validation evidence.

```sh
cargo test --frozen -p ferreth-conformance --test adr002_frame_gas
cargo test --frozen -p ferreth-conformance --test rc_revm_conformance
cargo test --frozen -p ferreth-compiler --test rv64_executor
cargo test --frozen -p ferreth-compiler --lib capacity -- --ignored --test-threads=1
cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --test-threads=1
```

Run the common gate as well. The ignored filters above select existing opt-in capacity probes. Record exact revision, commands, failures, skipped checks and resulting limits. Exit requires explicit evidence for all three layers and every vector.

## Implementation review

Check each increment against the accepted ABI, diagnostic codes, publication order and single execution implementation. Run the assigned corpus rows at RED and GREEN and retain prior semantic regression coverage. Phase 5 verifies the complete seven-opcode contract.

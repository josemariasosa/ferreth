# Phase 2.3d integration review

**READY FOR PHASE 2.4 REVIEW.** All six private opcodes passed the integration gate after three focused corrections. The composer still assembles one guest, arithmetic leaves match all 34 accepted words, and the observed ABI and dynamic metrics agree across comparable routes. The public cutover requires explicit artifact and diagnostic decisions. The four public LF-026 controls remain red. [Cutover plan](phase-2.4-cutover-plan.md)

## Baseline and question

Could the private path replace the public compiler and isolated arithmetic linker without changing supported semantics, guest publication or benchmark results? Success required independent expected outcomes, executable transfer checks, controlled rejections and no new workspace failures. The initial branch was `feat/symbolic-guest-composer`, HEAD `d7f1b9bf2a1da2dbc8438d5e35fd2658fa1038ba`. Status showed only the branch line and the cached diff was empty. The unmodified full suite accounted for 199 cases (192 passed, 4 failed, 3 ignored). All four failures occurred during public legacy display-source assembly.

The workspace has four members, with only ferreth-compiler a default member. The frozen lockfile selects asm-rs 0.2.0 (std/riscv, default features disabled), rvemu 0.0.11 and REVM 43.0.3. Verification ran on Darwin arm64 with Cargo/rustc 1.98.1. ADR-001 governs the code reservation, seeded registers, stack and publication. Proposed ADR-002 provides context for future gas work.

The evidence chain starts with the [Phase 0 capture and LF-026 reproducer](README.md), the [Phase 1 experiments and architecture decision](phase-1/results/1.7-architecture-decision.md), [2.0 capacity/readiness](phase-2.0-readiness.md) and [2.1 final-image controls](phase-2.1-final-image-verification.md). The [2.2 composer](phase-2.2-symbolic-guest-composer.md) and [2.3 boundary review](phase-2.3-boundary-review.md) explain the move to the [2.3a declaration contract](phase-2.3a-fragment-contract.md). The current comparison baseline incorporates [2.3b stack operations](phase-2.3b-pop-dup1.md), [2.3b.1 accepted limits](phase-2.3b.1-boundary-hardening.md) and [2.3c arithmetic](phase-2.3c-symbolic-arithmetic.md). Earlier proposals for fixed publisher inventories and POP capacity are superseded by those later decisions.

## Source ownership

Paths in this table are relative to `crates/ferreth-compiler/src/`.

| Responsibility | Source | Finding |
|---|---|---|
| Decode and lower | lib.rs:106, compiler/symbolic.rs:96 | Common decoder owns offsets, right-zero padding and first STOP. Private frontend dispatches by decoded instruction and supplies the next entry or Normal at EOF. |
| Opcode structure | compiler/symbolic.rs:150, rv64/opcodes/{push,pop,dup}.rs:33,43,55, rv64/opcodes/arithmetic.rs:8, lib.rs:228,263 | Generators own preflight, stack access, occurrence-local labels, branch operands, continuations, leaf calls/returns and publisher requirements. STOP is a frontend fragment factory. ADD/SUB select the existing leaf writers. |
| Fragment contract | compiler/symbolic.rs:49–94 | Source, entry, typed external role, local target and relative site word travel with the fixed-layout verification descriptor. A local continuation can coincide with a successor or Normal at EOF. |
| Composition and binding | compiler/symbolic.rs:166,183,196,207,216 | Check unique entry names, concatenate source, union external roles with mandatory Normal, append shared publishers, emit and finish once at ENTRY_PC. Bind PCs from assembler labels; checked word offsets locate the instructions inside each fragment. No opcode dispatch, byte patch, displacement rewriting or guessed label PC in compose. |
| Publishers | compiler/publisher.rs:24 | One source per role on the private path. Each publishes final s1, then exception code, then terminal status and exits through the sentinel JALR. |
| Final trust boundary | rv64/memory.rs:240, rv64/verify.rs:131,209,287,389,515,572 | Validate image base/size before reading final little-endian words. Compare decoded transfer and independent required layout, core and publisher contracts. Verification returns success/error; it produces no source, relocations, addresses for emission or repaired bytes. |
| Artifact and consumer | compiler.rs:35–76, compiler/symbolic.rs:216, orchestrator.rs:305,317, rv64/executor/rvemu.rs:118, rv64/guest.rs:588 | Construct CompiledRv64Program only after verification. The public consumer initializes memory/registers, executes to a supplied publisher PC, executes to the exit sentinel and reads published state. Private tests load the candidate's actual bytes into the same executor; production orchestration still selects legacy linkers. |

## Six-opcode execution matrix

Observed body counts exclude the 7-word Normal or 8-word exceptional publisher. Each occurrence's encoded transfers, alignment and end address pass both verifier stages. The runtime harness checks expected stack words, live and published s1, publication order, guards and sentinel exclusion. The following results combine `compiler/symbolic/tests.rs:312,340,377,420`, `tests/pop_dup1.rs:22,243,261,286` and `tests/arithmetic.rs:135,181,289,568`.

| Opcode | Fixed body and normal execution | Exceptional or boundary observation |
|---|---|---|
| STOP | 1 word, JAL x0 to Normal, body 1. Empty EOF begins at Normal, body 0. Trailing invalid and truncated instructions are unreachable. | Stack preserved, no exceptional role. |
| PUSH1 | 11 words, unsigned capacity check before stores, zero-extended immediate in four limbs, s1 +32, body 10. Truncated 60 equals 6000. | Full stack takes 4, Overflow code 1. From depth 1,023 the first push succeeds, the next preserves the full stack. |
| POP | 4 words, unsigned empty check, s1 −32 without a load/store, jump to successor, body 3. | At depth 0 takes 2, Underflow code 2. Repeated POPs and depths 1, 2 and 1,024 verify the remaining words. |
| DUP1 | 16 words, empty check before full check, four loads and four stores preserve the top word, s1 +32, body 14. | Empty takes 2 to Underflow, full takes 5 to Overflow. Two DUP1s from 1,023 preserve the first copy on the second fault. |
| ADD | 43 words including 34-word leaf. Two-word preflight, call at word 5 to word 9, return to word 6, retire 32 bytes, continue from word 7. Body 42. | Depth 0/1 takes 3 to Underflow without operand access. MAX+1 gives zero; propagation through three limbs and full-stack arithmetic pass. |
| SUB | Same wrapper shape, top minus next modulo 2²⁵⁶. The four-limb result occupies the lower slot. Body 42. | Same preflight and underflow count. [3,2] yields MAX; [1,2¹⁹²] yields 2¹⁹²−1. Repeated SUB and alternating ADD/SUB check distinct calls, anchors and operand order. |

**Artifact evidence.** Reassembly reproduces executable bytes. PUSH-bearing EOF artifacts match the comparable legacy bytes. With both exceptional roles, candidate Underflow precedes Overflow, reversing the corresponding legacy role addresses and trampoline encodings. An unused exceptional body costs 32 bytes on the legacy path. Publishers agree byte-for-byte by role. In the mixed public route each exception takes 8 instructions; the candidate static field counts all resident exceptional words. These are observable public migration issues. The [plan](phase-2.4-cutover-plan.md) specifies per-inventory handling.

**Arithmetic evidence.** Each call encodes 0x010000ef and each leaf ends in 0x00008067, returning to the exact decrement instruction 0xfe048493. `verify.rs:38` holds the 34-word baselines. Repeated embedded cores match the standalone bytes, including the return, with FNV-1a64 ADD 0xf1fff32ff47962f3 and SUB 0x9f008f320eeb7a7b. Guest execution runs instructions in rvemu. Host U256 calculations occur in the test oracle. Both singleton images match their isolated public counterpart across all 232 bytes with publisher addresses 0x800000ac and 0x800000c8. Their human-readable label spellings differ.

**Semantic and ABI evidence.** The 17 Phase 1.6 specimens and the expanded Phase 2.3 corpus passed on private and comparable legacy routes, including seeded full-width words, mixed EOF and STOP, prior successful writes before a later fault, preserved registers and guard bytes. Two exact Cancun fixture inputs extend that matrix: `600160000380` returns [MAX, MAX] at 76+7 instructions, and `6005800300` returns [0] at 67+7. The public Cancun differential independently compares the same bytecode against REVM, comparing every live stack word on success and termination class on exceptional halt. The seeded public 1,000 normal and 100 exceptional comparisons also passed. Those seeded cases do not directly execute the private frontend. REVM's known empty-DUP1 error detail is diagnostic. Private exceptional stack preservation is an ABI requirement. Phase 0 EELS Cancun (revision d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3) and direct REVM long-PUSH captures supply historical references; this review did not rerun the Python frame experiment or compare gas.

## Independent verification and its limits

The decoder reconstructs J-type signed 21-bit and B-type signed 13-bit displacements, adds them to the instruction PC with checked arithmetic and requires a complete aligned target word. It checks rd or funct3/rs1/rs2. Literal signed extremes, backward JAL −4, register swaps, a halfword target, wraparound and actual assembler relaxation exercise these checks. The declaration scan rejects duplicate, missing, extra or misaligned sites and unexpected JALR. The fixed-layout walk checks each required local call, return/update pairing, successor, exceptional role and exact ordered publisher bytes. It rejects trailing bytes and enforces a contiguous final inventory. Existing agreeing-declaration mutations swap DUP1 publishers, swap arithmetic calls/anchors, remove a required call, or redirect Underflow. The added POP mutation names STOP in both source and declaration and skips a live POP; the fixed-layout check rejects 0x80000020 where 0x80000010 was required.

These checks assume trusted lowering and the generator-selected fragment descriptor. They enforce the current fixed instruction widths, control families, arithmetic words and publisher words. Unspecified straight-line stack-operation words receive runtime/fixture coverage rather than exhaustive instruction semantics verification. This is directed evidence for valid ADR-001 entry state, not formal verification of arbitrary RV64 inputs.

## Reproduced defects

**RF-023 — Unsupported PUSH diagnostic order.** For `61 00 01 62` public compilation reported PUSH2 at offset 0. Private lowering reported PUSH3 at offset 3 because it inspected the continuation before lowering the current instruction. The new `unsupported_push_diagnostics_follow_decoded_order` regression failed on the initial implementation. Preserve the decoder's existing error precedence and request a continuation only for an instruction that uses it.

**RF-024 — Zero in the publisher PC inventory.** The standalone verifier treated address zero as a missing optional role even when the caller explicitly supplied the role. A valid 28-byte Normal body, with the final Overflow body removed and its declared address set to zero, must fail the Normal/Overflow contract. The old filter discarded zero. The new `declared_publisher_at_zero_cannot_hide_a_missing_body` regression requires rejection. Production composition obtains these addresses from assembler labels in the ADR-001 code region. This defect concerns the verifier's independent metadata check.

**RF-025 — Transfer declarations lost fragment ownership during binding.** Moving word 10 of the second PUSH1 into the first fragment's declaration list as word 21 left every absolute site, target and byte unchanged. Composition returned a program. The single-case test `declaration_site_must_belong_to_its_fragment` failed before the correction. A transfer site's owner must contain that instruction even when another occurrence uses the same publisher. A target may legitimately lie outside its owner.

All three fixes and their regressions are implemented. RF-023 defers successor lookup until the current instruction requires it. RF-024 iterates every supplied publisher address, retaining optionality in the role itself. RF-025 bounds both local and external declaration sites by the next resolved entry or Normal. The subsequent global scan still checks absolute coverage. None of the fixes changes a valid guest's source or encoding.

## Resource and failure policy

The three existing opt-in probes passed. The executable interval is [0x80000000, 0x800ffffc), permitting 1,048,572 bytes. asm-rs applies its pinned defaults independently and can report an error before layout validation. The following are implementation capacities.

| Specimen | Last accepted | Controlled rejection |
|---|---|---|
| POP at EOF | 49,999, exactly 100,000 labels, 800,044 bytes; seeded execution reaches underflow after 1,024 retirements | 50,000 requests 100,002 labels. Link/ResourceLimitExceeded, despite nominal 800,060 bytes fitting ADR-001. RF-020 accepted. |
| DUP1 at EOF | 16,382, 1,048,540 bytes; first and later exceptional exits exercised | 16,383 assembles to 1,048,604 bytes then Layout/CodeOverlapsExitSlot. At 16,384 the JAL displacement +1,048,576 fails during assembly. RF-019 closed outside the executable domain. |
| PUSH1 at EOF / STOP | 23,829, respectively 1,048,536 / 1,048,540 bytes | 23,830, respectively 1,048,580 / 1,048,584 bytes, Layout/CodeOverlapsExitSlot. |

The probes exercise the component returning each failure. Compiler error propagation through assemble/verify returns a Result before constructing an artifact. No partial program or panic was observed on these vectors. General allocator exhaustion was outside the experiment.

## Verification ledger

Commands used the workspace root and frozen lockfile with the existing target directory. Counts include all workspace test binaries and doctests. Separate focused/opt-in runs are excluded from workspace totals.

| Command | Observed result |
|---|---|
| Baseline `cargo test --frozen --workspace --no-fail-fast` | 192 passed, 4 failed, 3 ignored, 0 measured, 0 filtered; 199 total. Exit 101. |
| `cargo fmt --all -- --check` | Pass. |
| `cargo clippy --frozen --workspace --all-targets -- -D warnings` | Pass. |
| `cargo test --frozen -p ferreth-compiler --lib` | 83 passed, 0 failed, 2 ignored. Includes all private and verifier regressions. |
| `cargo test --frozen -p ferreth-linking-spike` | 22 passed, 0 failed, 1 ignored. |
| `cargo test --frozen --workspace -- --list` | 205 tests, 0 benchmarks. Six added since baseline. |
| `cargo test --frozen --workspace --no-fail-fast` | 198 passed, 4 failed, 3 ignored, 0 measured, 0 filtered. Exit 101. |
| `cargo test --frozen -p ferreth-compiler --lib capacity -- --ignored --nocapture --test-threads=1` | Both compiler probes passed, 83 filtered. |
| `cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1` | 1 passed. |
| `cargo run --frozen -q -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json` | Both 1,012/1,012, 34 words and 34 executed per case, 34,408 per core. Parsed report equals runs/rv64-arithmetic.json in full. |
| `cargo run --frozen -q -p ferreth-conformance -- bench guest --format json` | Five isolated and ten program measurements. Parsed report equals runs/phase-0/guest-benchmarks.json in full. |

Both benchmark outputs carried the known rvemu device-tree warnings before the opening JSON object (arithmetic 4 lines/326 bytes, guest 30 lines/2,445 bytes). Only those prefixes were excluded from the parsed-object comparisons. No tracked baseline was regenerated.

The four failures below appeared in discovery and failed before execution in `crates/ferreth-compiler/tests/rv64_push1_program.rs`, unchanged from HEAD. Each reported `ImmediateOverflow` at line 12, column 5, with bounds −2048 through +2047.

| Public LF-026 control | Legacy displacement |
|---|---:|
| repeated_push1_47_eof_completes_normally | +2056 |
| repeated_push1_47_eof_full_stack_publishes_overflow | +2056 |
| repeated_push1_47_stop_completes_normally | +2060 |
| repeated_push1_47_stop_full_stack_publishes_overflow | +2060 |

The private 46/47 EOF and STOP controls decoded the first long JAL and executed both normal and full-stack overflow paths. For 47 pushes the first words at entry+40 are 0x0090006f and 0x00d0006f. Normal takes 470+7 or 471+7, overflow takes 4+8. The public legacy failure and the private result are separate observations.

## Changed-file inventory

| Path | Review change |
|---|---|
| crates/ferreth-compiler/src/compiler/symbolic.rs | Private diagnostic ordering and declaration-site ownership. |
| crates/ferreth-compiler/src/compiler/symbolic/tests.rs | Error precedence, agreeing wrong POP continuation and cross-fragment declaration regressions. |
| crates/ferreth-compiler/src/compiler/symbolic/tests/arithmetic.rs | Isolated whole-image comparison and two shared Cancun vectors. |
| crates/ferreth-compiler/src/rv64/verify.rs | Check every declared publisher PC. |
| crates/ferreth-compiler/src/rv64/verify/tests.rs | Missing-body/zero-PC regression. |
| research/guest-linking/phase-2.3d-integration-review.md | This review and RF-023 through RF-025. |
| research/guest-linking/phase-2.4-cutover-plan.md | Proposed routing, compatibility decisions, gates and rollback. |
| research/guest-linking/README.md | Research index. |
| docs/research/ferreth-linking-architecture.md | Phase 2 roadmap and findings. |

Unrelated untracked PNG files appeared and changed names under `assets/` after the clean baseline. The inventory above excludes these concurrent workspace changes. This audit made no edits under `assets/`.

Local links in the new deliverables resolve. The final tracked diff passed `git diff --check`, the index was empty, and the LF-026 test file matched HEAD.

**Architectural judgment.** The private integration satisfies this gate. Current residual risk is concentrated at the public boundary: callers can observe label text, role order, exceptional static counts and errors, and some existing tests intentionally use the public function as a legacy oracle. The plan identifies those dependencies before any route changes. Maintainer approval of the bounded cutover and metadata policy is the next decision.

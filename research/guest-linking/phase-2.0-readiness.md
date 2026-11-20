# Phase 2.0 — Production migration readiness

**Status:** Readiness investigation complete; no production migration is authorized by this report.  
**Target:** One symbolic guest composer using the frozen `asm-rs 0.2.0` resolution, with independent checks of final machine bytes against intended control-flow roles.  
**Authority:** Accepted [ADR-001](../../docs/adr/001-evm-guest-state-abi.md), proposed [ADR-002](../../docs/adr/002-cancun-evm-gas-metering.md), and the reviewed [Phase 1.7 decision](phase-1/results/1.7-architecture-decision.md). ADR-002 remains proposed and does not authorize gas or opcode work.

## 1. Scope and findings

This phase answers migration-readiness questions only. It does not change production or conformance code, compiler routing, guest behavior, dependencies, or the four LF-026 regressions. The only executable addition is an ignored, explicit research probe at [capacity_probe.rs](phase-1/tests/capacity_probe.rs); the probe and its command are reproducible without adding a dependency.

**Observed:** Phase 1.1–1.6 established symbolic JAL resolution, final labels and bytes, independent transfer decoding, execution of 47-PUSH guests including a taken overflow transfer, mixed-operation parity, and unchanged standalone arithmetic core hashes. The production workspace still rejects the same four LF-026 cases. See the [Phase 1 evidence index](phase-1/README.md).

**Readiness risks requiring resolution before implementation:**

1. `asm-rs` defaults to 100,000 labels. Its label limit can reject a symbolic guest whose code is well within Ferreth's executable region; the exact result depends on the composer's label density.
2. The Phase 1.7 proposed POP maximum assumes a 20-byte body, but the current production and Phase 1.6 symbolic POP bodies are 16 bytes. The Phase 1.6-style composition also needs three labels per POP, which reaches the default label cap before the Ferreth image-size ceiling.
3. Public artifacts and CLI commands expose assembly, bytes, publisher PCs, and metrics. History supports short-entry-point equality and arithmetic-core identity, but there is no inventory of external consumers requiring every historical artifact to remain byte-identical.
4. Isolated ADD/SUB currently have two publishers. Routing through the ordinary mixed guest inventory adds the overflow publisher and changes static size and underflow PC. This is measured in Phase 1.7, but still needs explicit compatibility approval.

## 2. Current ownership and removal inventory

| Responsibility | Current owner and behavior | Candidate disposition after parity |
|---|---|---|
| PUSH1-only whole guest | [`compile_push1_program`](../../crates/ferreth-compiler/src/compiler.rs) generates fragments, places normal then overflow, calculates numeric JALs, patches bytes and reconstructs source. `compile_evm_bytecode` selects it for PUSH1-only EOF input, including empty input; the sequence API calls it directly. | Make the sequence API an adapter to the common symbolic composition. Retain the public API and unlinked PUSH1 generator. |
| Mixed/STOP whole guest | Private `compile_supported_program` in [`compiler.rs`](../../crates/ferreth-compiler/src/compiler.rs) handles PUSH1, ADD, SUB, POP, DUP1 and explicit STOP. It lays out the normal/overflow/underflow publishers, patches each body's external sites, rebuilds source and constructs `CompiledRv64Program`. | Use one composer for all lowered supported programs. Keep EVM decoding/lowering and STOP/truncation behavior unchanged. |
| Isolated ADD/SUB guest | [`link_guest_arithmetic`](../../crates/ferreth-compiler/src/orchestrator.rs) is one shared linker called by ADD and SUB adapters. It places normal and underflow exits, patches one trampoline, rewrites source and returns a private linked value consumed by the existing executor. | Converge these guest adapters on ordinary single-op bytecode compilation only after review of the publisher-layout delta. Keep `Rv64Runner`, `ProgramRunner`, the standalone pointer cores and their separate `opcode_core` contract. |
| Publisher source | The compiler owns `normal_exit_assembly`, `overflow_exit_assembly`, and `underflow_exit_assembly`. The orchestrator separately owns `arithmetic_normal_exit_assembly` and `arithmetic_underflow_exit_assembly`. Normal and underflow publication are duplicated; overflow currently exists only in the compiler. All implement the same 24-byte ADR-001 prefix, status-last order and sentinel transfer. | One guest publisher emitter, with layout and stable codes still subject to the accepted ABI and future maintainer decisions. Do not change the record or codes in this migration. |
| JAL encoding and placement | `relative_jal_offset` and `encode_jal_zero` are duplicated in compiler and orchestrator. LF-027 demonstrates that both encoders mishandle the sign bit for negative displacements. Whole-program code also renders numeric JAL operands, which hit the assembler's signed 12-bit numeric parser at LF-026. | Symbolic external JALs remove the two handwritten encoders if the migration is approved. Preserve independent byte decoding; do not treat relocation records as proof of intent. |
| Text reconstruction | PUSH1 strips a fixed source prefix/suffix. Mixed linking uses `linked_add_assembly`, `linked_sub_assembly`, `linked_pop_assembly`, and `linked_dup1_assembly` string replacements. Isolated arithmetic replaces an exact trampoline string. These helpers rename labels and replace placeholder JALs in parallel with byte patches. | Remove only after the symbolic candidate passes the old/new gates; keep display text reassembly as a consistency check, not as the independent verifier. |
| Artifact and execution boundary | `CompiledRv64Program` exposes assembly, machine bytes, entry/exit PCs, instruction limit, publisher PCs and termination-count information. `execute_linked_program` then initializes and executes the same ABI through `execute_raw_program`; runtime result validation is already shared. | Preserve artifact and executor boundaries. Do not move linking decisions into execution or alter execution semantics. |

`compile_supported_program` already checks selected generator offsets/words and both whole-program paths compare patched machine bytes with assembly reassembly. Those checks establish byte/source consistency, not that an emitted external JAL reaches the intended semantic publisher.

## 3. Pinned assembler limits and observed capacity

### Configuration

The Phase 1 crate and production compiler resolve `asm-rs 0.2.0`; its lockfile checksum is recorded in [Phase 1.1](phase-1/results/1.1-assembler-api.md). The builder exposes `ResourceLimits` through `Assembler::limits`. The defaults below are source-derived from pinned `asm-rs 0.2.0` `src/assembler.rs` and were also printed by the capacity probe:

| Limit | Default | Relevance to guest composition |
|---|---:|---|
| Parsed statements | 1,000,000 | Instructions and labels count. Below this default at the tested PUSH image boundary and POP label boundary. |
| Defined labels | 100,000 | Binding for symbol-dense guests; experimentally confirmed at both 100,000 and 100,001. |
| Output | 16,777,216 bytes (16 MiB) | Much larger than Ferreth's 1,048,572-byte executable ceiling. It does not enforce the guest ABI bound. |
| Source per `emit` | 67,108,864 bytes (64 MiB) | Tested candidate PUSH source is 7,713,375 bytes at the first Ferreth-overbound case. |
| Preprocessor-expanded source | 67,108,864 bytes (64 MiB) | No generated Phase 1 guest uses preprocessor expansion. |
| Preprocessor iterations | 100,000 | Not exercised by the generated guest format. |
| Macro recursion depth | 32 | Not exercised by the generated guest format. |
| Accumulated errors | 64 | Error-reporting guard, not a guest-capacity target. |

The probe empirically verified the exact default label boundary: 100,000 named labels plus one RV64 instruction assemble to 4 output bytes; a source with 100,001 labels returns `ResourceLimitExceeded { resource: "labels", limit: 100000 }`. Its input was 1,288,923 source bytes. Small overrides also returned the expected source-byte, statement and output-byte resource errors. Those override checks verify that the corresponding configuration fields are effective; they do not claim to exhaust the defaults for those larger limits.

### Guest-size measurements

Ferreth's accepted RV64 code interval is `[ENTRY_PC, EXIT_PC)`, excluding the four-byte exit slot: maximum generated size is **1,048,572 bytes** (`EXIT_PC - ENTRY_PC`). `validate_generated_code` enforces this after assembly. It is separate from assembler reach and assembler resource limits.

The Phase 1.5 PUSH1 fixture uses 44 bytes per PUSH, 60 publisher bytes, and three labels per PUSH plus two publisher labels. The capacity probe assembled complete symbolic sources at the guest origin:

| PUSH1 count | Source bytes | Labels | Assembler output | Ferreth layout |
|---:|---:|---:|---:|---|
| 1 | 651 | 5 | 104 bytes | Accept |
| 47 | 14,912 | 143 | 2,128 bytes | Accept |
| 23,829 | 7,713,049 | 71,489 | 1,048,536 bytes | Accept |
| 23,830 | 7,713,375 | 71,492 | 1,048,580 bytes | Reject `CodeOverlapsExitSlot`; assembler succeeds |

Thus, for this tested PUSH layout, Ferreth's code bound is reached first; the assembler output default is not. The 23,829/23,830 pair separates Ferreth's executable rejection from assembler rejection. The successful 47-PUSH research run separately proves long-JAL layout and execution; these near-bound probes assemble and validate layout, not execute a 1-MiB guest.

For the Phase 1.6-style POP body, each occurrence is four instructions (16 bytes), with an entry, underflow and continuation label. The three publisher bodies contribute 92 bytes and three labels. The probe observed:

| POP count | Source bytes | Labels | Assembler result | Projected/actual image |
|---:|---:|---:|---|---:|
| 33,332 | 6,311,398 | 99,999 | Success | 533,404 bytes; Ferreth accepts |
| 33,333 | 6,311,589 | 100,002 | Rejects at 100,001st label | 533,420 bytes if assembled; below Ferreth's ceiling |

This is a demonstrated assembler-label rejection before the guest code bound. With 16-byte bodies and 92 publisher bytes, the image-only arithmetic would reach 1,048,572 bytes at 65,530 POPs, but the measured three-label-per-operation representation cannot approach that size under the default label limit. The supported maximum therefore depends on the selected symbol/manifest representation, not only fragment byte widths.

### Phase 1.7 sizing discrepancy — RF-016

The Phase 1.7 proposal asks to compile 52,424 POPs at 20 bytes each plus 92 publisher bytes, yielding 1,048,572 bytes. The inspected production [`generate_guest_pop`](../../crates/ferreth-compiler/src/rv64/opcodes/pop.rs) is 16 bytes, and the Phase 1.6 symbolic POP wrapper is also 16 bytes. The 20-byte assumption is not substantiated by that evidence. Further, three labels per POP at 52,424 operations would require 157,275 labels, above the pinned default; even two per POP plus three publishers would exceed it. This proposal is a sizing/gate inconsistency, not evidence that symbolic composition cannot support a larger guest.

**Required disposition:** Before Phase 2 implementation, maintainers must approve the actual generated layout and a symbolic label budget. Revise the 52,424 target to a source-derived case, reduce label density with an independently verified representation, or explicitly approve a bounded assembler-limit policy change after resource measurements. Do not silently raise limits, weaken Ferreth's executable bound, or claim the current layout meets the proposed gate.

### Reproduction

From the repository root, using the frozen workspace lockfile:

```sh
CARGO_TARGET_DIR=target/phase2-baseline \
  cargo test --frozen -p ferreth-linking-spike --test capacity_probe \
  -- --ignored --nocapture --test-threads=1
```

The test is intentionally ignored in the normal workspace suite because it creates 100,000 labels and multi-megabyte source strings. Its recorded run completed successfully in 2.95 seconds on the current host. Source-level defaults are in pinned `asm-rs 0.2.0` `src/assembler.rs` (`ResourceLimits`, `Assembler::limits`, `emit`, `process_statement`, `finish`); symbol counting occurs on parsed labels, while output is checked separately.

## 4. Compatibility inventory and decisions for approval

| Surface | Existing compatibility evidence/requirement | Approval needed before a change |
|---|---|---|
| EVM decoding and lowering | First STOP ends execution decoding; truncated PUSH is zero-padded; PUSH2–PUSH32 decode but compilation rejects them with the original byte offset; unsupported opcodes remain compile/decode errors. Full-stream `decode`/disassembly intentionally differs from execution-prefix handling. | No semantic, support-set, offset, or error-category changes in Phase 2. |
| Guest behavior and ABI | Preserve the accepted 1,024-word ascending stack, `s0/s1`, status/code/final-`s1` record, status-last stores, deliberate sentinel jump, and separation of guest exceptions from executor faults/step limits. | No ABI fields/codes, publisher ordering, register conventions, or execution policy changes. |
| Standalone arithmetic cores | ADD/SUB leaf code/hash and pointer convention are protected by the 1,012-case benchmark: 34 static and 34 dynamic words per case; hashes `0xf1fff32ff47962f3` and `0x9f008f320eeb7a7b`. | No core-byte, arithmetic, operand-order, or benchmark-boundary changes. |
| Public compile APIs | `compile_evm_bytecode` and `compile_push1_program` are public. Existing tests require equivalent PUSH1 bytecode/sequence inputs to yield equal machine bytes and assembly. `CompileError` is public. | Retain signatures, result/error categories, bytecode offsets, bytecode/sequence equality, and the compile-without-execution contract. |
| Public guest APIs/results | PUSH1, sequence, ADD/SUB (including seeded variants), POP/DUP1, and bytecode guest execution entry points are public. `OrchestratorResult` exposes assembly, bytes, guest result, and metrics. | Retain these adapters and observable result semantics; do not make initial stack part of compilation. |
| Fragment and CLI output | Public generators provide unlinked fragment assembly/words. `translate` prints arithmetic leaf source or PUSH1 fragment plus words. `execute --push1-sequence` prints linked assembly and split metrics; other commands print their documented result fields. | Exact whole-guest text is not established as an immutable golden, but assembly is public/printed. Confirm downstream consumers before accepting text/byte/publisher-layout deltas. Preserve the fragment contract and documented CLI output shape. |
| Publisher inventory and metrics | PUSH1-only EOF currently has normal+overflow; mixed/STOP has normal+overflow+underflow; isolated arithmetic has normal+underflow. Mixed exception publishers are eight instructions. `CompiledRv64Program` exposes ordered publisher PCs and a single static exceptional count; runtime metrics count the observed selected publisher. | Approve whether isolated ADD/SUB adopt the three-publisher inventory. Phase 1.7 estimates a 232-to-264-byte artifact change and an underflow PC shift from `0x800000c8` to `0x800000e8`; verify these exact values against the candidate and explicitly accept them before routing. |
| Historical layouts | `compile_push1_program` was retained when mixed compilation was added. Existing tests assert short artifact equality across public PUSH1 inputs and one explicit STOP text pattern, not a permanent golden for every historical guest. | Review every differing byte size, publisher PC, static count and CLI text. Replace a brittle text assertion with an independent final-word/target assertion only in the approved implementation chunk. |

No downstream consumer inventory is present in the repository. The maintainer's Phase 1.7 direction permits guest text changes in principle but requires explaining artifact differences; it does not establish that no external user depends on the currently exposed strings or layout.

## 5. Safe boundary for independent final-image verification

The verifier can be a pure compile-time check and need not change generated execution:

1. Build the candidate source/reference manifest. Keep operation semantics, instruction order, local checks, core bytes and publisher instruction sequences unchanged.
2. Assemble with the pinned builder at `ENTRY_PC`, obtaining final bytes and labels. Do not infer final PCs from source line count or relocation offsets.
3. Before constructing `CompiledRv64Program` (or returning the isolated linked artifact), validate the final image: instruction alignment and full-word bounds; image end before `EXIT_PC`; required publisher labels, starts and ordering; expected declaration count/uniqueness; all declared JAL sites, `rd=x0`, signed displacement and exact target role; local branch fields/targets; ADD/SUB call and return anchors; publisher metadata stores, status-last order and sentinel transfer. Reject unexpected relaxed local branches until their encoding and both paths are independently checked.
4. Preserve assembly-to-byte reassembly equality as a presentation/round-trip check. It is not independent: the verifier must decode little-endian final words and compare against separately retained intended sites/roles. Directed execution fixtures must independently assert which exception is expected, so a consistently wrong declaration cannot validate itself.
5. Continue using the same artifact fields at the existing `execute_linked_program`/`execute_raw_program` boundary. The executor continues to load and execute the exact validated bytes, and `read_guest_result` remains the ABI authority.

The same checkpoint can be reached in the current production routes without changing guest semantics: after final patched bytes and display reassembly are available, and before `CompiledRv64Program` construction in `compiler.rs`, or before `LinkedGuestArithmetic` is returned in `orchestrator.rs`. The symbolic candidate should instead verify `AssemblyResult.bytes()` and labels directly. In both cases a mismatch is a compile/link error before execution, not a guest exception.

## 6. Recommended implementation chunks

Proceed only after maintainers review the architecture, the RF-016 size correction, and compatibility decisions above. Keep the four LF-026 tests active as RED controls until the production route changes; do not skip, weaken, or rewrite their expected behavior.

| Chunk | Bounded scope | Acceptance | Stop/rollback condition |
|---|---|---|---|
| **2.1 Final-image verifier** | Add private verification under `crates/ferreth-compiler/src/rv64/`; keep all production linking routes unchanged. Add independent literal/assembler fixtures and wrong-target controls. | Port RF-008–011 coverage: signed JAL limits and −4; aligned Ferreth targets; reject halfword-aligned assembler-valid target, wrong target/opcode/`rd`/branch fields, incomplete site/target, address overflow, missing/duplicate declarations and unexpected expansion. Verify exact code-end acceptance and overlap rejection. Existing 4 LF-026 failures remain unchanged. | Stop if a wrong intended target passes or a valid current image fails. Remove/revert only the verifier chunk before routing work. |
| **2.2 PUSH1/STOP candidate beside legacy** | Add the symbolic PUSH1/STOP composition and shared publication candidate. Keep `compile_push1_program` and bytecode routing on legacy while comparing candidates through a private test seam or isolated frozen comparison. | Compare empty, one/two/five PUSH, EOF/STOP, truncated `60`, first STOP with trailing bytes, 46 controls and the 47 normal/full-stack cases. Require short artifact parity where observed, independent far-JAL checks, taken overflow, stack/guard preservation, reassembly and exact 23,829/23,830 size behavior. | Keep legacy routing if target, bytes/result, publisher order, ABI or layout differs without explicit approval. Do not let the 47 cases disappear as compile-time skips. |
| **2.3 Mixed composition and compile cutover** | Add ADD/SUB/POP/DUP1 emitters to the composer; route both public compile APIs only after side-by-side acceptance. Correct the POP-size/label budget gate first. | Replay all 17 Phase 1.6 specimen pairs and both DUP1 errors, repeated arithmetic call/return paths, underflow, overflow, carry/borrow and guards. Require standalone core identity. Make all four LF-026 tests pass on the production route. Measure a real POP threshold from its actual emitted source and chosen label strategy; do not use 52,424/20 without evidence. | Restore legacy compile routing if a semantic/conformance regression, wrong site, unapproved layout/API delta, label-limit surprise, or unexplained artifact change appears. Keep candidate tests and RED controls. |
| **2.4 Isolated ADD/SUB convergence** | Change only seeded isolated ADD/SUB adapters to use ordinary one-op bytecode compilation and the existing artifact executor. | Compare success, top-minus-next SUB, zero/one-word underflow, published status/code/s1, all stack words, split dynamic counts, core hashes and the reviewed publisher-size/PC delta. Keep public adapter signatures. | Keep isolated routing on legacy until static/layout delta is accepted and all result/metric/fault checks pass. |
| **2.5 Remove legacy linking** | Remove the obsolete compiler/orchestrator linkers, duplicate publisher source, handwritten JAL encoders and string reconstruction only after review of parity evidence. No cleanup bundled into earlier chunks. | One guest-composition and publication owner; all gates below pass; standalone cores/fragments and public APIs remain. Update directly related research/docs for actual accepted layout. | Restore the last reviewed comparison-capable state if any gate regresses. Stop before deleting a path whose parity evidence is incomplete. |

## 7. Required regression and conformance gates

- **LF-026 unchanged until repaired:** preserve all four test names and their semantics: 47 PUSH1 EOF/STOP normal completion and seeded full-stack overflow. Candidate empty runs must publish NORMAL/code 0 with 47 twos and `s1=0x801015e0`; seeded overflow runs must reach the independently decoded publisher, publish GUEST_EXCEPTION/code 1, preserve all 1,024 words and `s1=0x80109000`, and leave the guard intact. Keep the 46-PUSH EOF/STOP passing controls.
- **Transfer verification:** decode final bytes independently at every declared external and local site; check instruction opcode/fields, `rd`, signed immediate, PC-relative target, alignment and full-word image bounds. Include the LF-027 negative-JAL word and wrong-target/wrong-role controls. Verify all sites are accounted for; relocations and reassembly alone are insufficient.
- **Composition parity:** run the Phase 1.4 short PUSH and Phase 1.6 mixed cases through production entry points; include empty/seeded stacks, EOF/STOP, first STOP/trailing input, truncated PUSH1, 1,023/1,024-depth boundaries, all exceptional exits, repeated ADD/SUB leaves and both DUP1 exits. Compare status/code, live stack, live/published `s1`, ABI registers, guards, publisher PC, and body/termination counts.
- **ABI and error separation:** retain hand-authored publication tests, status-last checkpoints, sentinel pre-fetch behavior, unchanged guest exception codes, and distinct compile/link errors, malformed result, emulator error and step-limit outcomes.
- **Public compatibility:** test `compile_evm_bytecode` versus `compile_push1_program` equality; verify `CompiledRv64Program` metadata/entry bounds and the existing convenience APIs. Review CLI commands that print assembly/words/status/stack/counts. Record accepted assembly, publisher, static-size and publisher-PC deltas explicitly.
- **Standalone arithmetic and conformance:** preserve ADD/SUB core bytes, hashes, 1,012/1,012 benchmark cases and 34-word per-case metrics. Keep current directed and seeded REVM Cancun tests, including 1,000 valid and 100 exceptional generated cases. Gas and new opcodes are excluded.
- **Capacity:** retain an explicit boundary test for assembler label cap versus Ferreth's code cap. Test the final approved symbolic manifest's actual label count, generated source size, output size and acceptance/rejection boundary. Do not infer maximum guest length from opcode-bytecode length alone.
- **Final workspace gate after implementation:** run frozen format, strict workspace Clippy, complete workspace tests and test discovery in a fresh target directory. Reconcile discovery with execution; at final cutover all production tests and Phase 1 research tests must pass (apart from any separately approved ignored research probe), with no unexplained omissions. Re-run the arithmetic benchmark and preserve raw output if rvemu warning prefixes affect JSON capture.

## 8. Baseline, verification, and stop conditions

Before adding the capacity probe, the full frozen workspace run in this checkout executed **163 tests: 159 passed, 4 failed, 0 ignored**. Those are the 141 production tests plus the 22 accepted Phase 1 research tests. The only four failures were the unchanged LF-026 cases:

| Test | Rejected numeric JAL |
|---|---:|
| `repeated_push1_47_eof_completes_normally` | +2056 |
| `repeated_push1_47_eof_full_stack_publishes_overflow` | +2056 |
| `repeated_push1_47_stop_completes_normally` | +2060 |
| `repeated_push1_47_stop_full_stack_publishes_overflow` | +2060 |

Each fails before guest execution with `ImmediateOverflow`, bounds `-2048..=2047`. The passing 46-PUSH controls and all conformance tests remain present. After adding the ignored capacity probe, workspace discovery reports 164 test cases. The post-probe workspace run confirmed 159 passed, the same four LF-026 failures, and one ignored capacity experiment. The Phase 0 inventory remains the source of the historical RED baseline.

Checks actually run for this readiness phase:

- Full pre-probe workspace tests: exit 101 with 159 passed and the four named LF-026 failures; no other failure.
- Full post-probe workspace tests: the same four LF-026 failures, 159 passed, and the capacity experiment ignored.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --frozen -p ferreth-linking-spike --all-targets -- -D warnings`: passed.
- `cargo test --frozen -p ferreth-linking-spike -- --test-threads=1`: passed all 22 Phase 1 tests; the capacity experiment is ignored by default.
- `CARGO_TARGET_DIR=target/phase2-baseline cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1`: passed, including the measured assembler and Ferreth boundaries above.
- `cargo test --frozen --workspace -- --list`: discovery includes the new ignored experiment; the compiler's four LF-026 names are still discoverable.
- `git diff --check`: passed.

**Stop conditions for any next phase:** no maintainer approval for symbolic composition; unresolved RF-016 layout/label budget; any unauthorized dependency/ABI/API change; verifier false acceptance or rejection; any unexplained semantic, publisher, byte/metric, or CLI difference; loss of any LF-026 regression; or an assembler resource failure before the approved supported-guest boundary. Do not add gas, new opcodes, or production code during readiness.

The [Phase 1.7 migration proposal](phase-1/results/1.7-architecture-decision.md#e-proposed-phase-2-migration) remains the architectural starting point, with the POP capacity target corrected before implementation. Phase 2.0 is complete at this report; the next action is maintainer review and explicit authorization of a bounded 2.1 implementation.

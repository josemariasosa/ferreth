# Phase 2.4 public cutover plan

**Phase 2.4 cutover gate.** The [Phase 2.3d integration review](phase-2.3d-integration-review.md) gives the private baseline. The table records the original compatibility proposal with the maintainer-approved RF-027 static-count policy. The [public cutover review](phase-2.4-public-cutover.md) records current verification.

## Compatibility decisions

| Observable | Public cutover policy |
|---|---|
| Input and errors | Preserve signatures, decode the reachable prefix through the first STOP, zero-fill truncated PUSH1, accept EOF including empty, reject decoded PUSH2–32 with the first unsupported instruction's opcode and byte offset. Preserve decode-stage precedence for an unknown reachable opcode. Exercise RF-023. Assembler/default-limit and final verification errors return Link, ADR-001 image violations return Layout. Preserve error context. The obsolete numeric-JAL parser failure disappears for valid long inputs. |
| Assembly text | Return exactly the symbolic source used to assemble the loaded bytes. Labels and numeric displacements in legacy display text change. Keep deterministic rendering, round-trip equality, one-instruction STOP and no unresolved trampolines. |
| Publisher PCs and bytes | At the end of the body, P, emit Normal. Append Underflow if declared, then Overflow if declared. Each exceptional body occupies 32 bytes. Publish the actual entry PCs in physical order. Mixed legacy emits Normal, Overflow, Underflow even for unused exits. Consumers that assign roles by index need a layout-aware expectation. The executor accepts the entire stop-PC set. Do not publish nonexistent addresses for compatibility. |
| Static counts | Keep entry, sentinel and image-length/4 instruction limit. Normal occupies 7 words. Under RF-027, the maintainer approved `exceptional_termination_instructions()` as the number of resident exceptional publisher words. The count is 0 with neither publisher, 8 with Underflow or Overflow alone, and 16 with both. |
| Dynamic counts and ABI | Preserve actual body and selected publisher counts, total, live/published pointer, status, exception code, stack and pre-fetch sentinel. Successful execution adds 7 publisher instructions, faults add 8. Preserve the ADR-001 standalone pointer-ABI benchmark. |

For nonempty PUSH1 at EOF, the existing two role addresses and bytes already agree. With STOP after only PUSH1, the legacy mixed route appends an unused Underflow. Empty input shrinks from 60 to 28 bytes, STOP alone from 96 to 32. Arithmetic/POP with only Underflow shrinks the legacy mixed image by 32 bytes. Bodies requiring both exceptional roles keep their length and move the exception JAL targets to the corresponding new PCs. The isolated arithmetic linker already uses Normal/Underflow at P/P+28. Full isolated images are byte-identical today. Assembly text differs.

## Smallest reviewable sequence

1. **Preserve the oracle before redirecting public calls.** Keep named legacy implementations accessible to compiler unit comparisons during migration. Update the private comparison helpers in `compiler/symbolic/tests.rs`, `tests/pop_dup1.rs` and `tests/arithmetic.rs` to call those legacy implementations. The 47-PUSH and ignored capacity helpers currently expect errors *from the public function* as their legacy control. Move those expectations to the legacy seam. Bind the legacy text mutations in `rv64/verify/tests.rs:228` and mixed metadata fixture at line 285 to a legacy fixture. Preserve each rejection and add public checks for the symbolic layout. Avoid redirecting an oracle back to the candidate.
2. **Route the two compile APIs together.** Today `compiler.rs:119` decodes, then calls `compile_push1_program` at line 574 for pure EOF pushes, or private `compile_supported_program` at line 177 for mixed/STOP. Have the bytecode API call the private symbolic frontend once. Give the immediate-slice API a sibling private adapter that enumerates the supplied bytes as PUSH1 immediates and calls the same composer, including for an empty slice. Never decode immediates as opcodes. Apply the approved public metadata policy once at the artifact boundary. Keep the legacy mixed assembler available for regression/rollback. The bytecode, sequence, POP and DUP1 orchestrator facades inherit this switch.
3. **Migrate representation-specific tests in the same change.** In `tests/rv64_push1_program.rs:167` replace the literal numeric STOP assembly substring with independent verification of the one-word JAL x0 to Normal and reassembly. Keep all four LF-026 tests and their helpers intact. In `tests/rv64_final_image.rs` assert the role of each publisher body as well as its PC (the same PC list can encode swapped roles). The Phase 1.6 comparison at `phase-1/tests/mixed_symbolic_guest.rs:612` calls the public compiler. Its production layout checker expects legacy label names and three publishers unconditionally. Teach its production-side checker the new entry names and role inventory while retaining independent decoded targets, terminal results and split counts for all 17 vectors. Keep the original experimental assembler and its recorded results. Check CLI execute output for assembly, bytes, result and counts.
4. **Then converge seeded isolated arithmetic.** Replace only the linking part of `execute_guest_add_with_initial_stack` and `execute_guest_sub_with_initial_stack` (`orchestrator.rs:219,243`) with ordinary singleton compilation and `execute_linked_program`. Keep the two-operand adapters, seed order, depth rejection and executor/result error variants. Map internal Link/Layout errors back to the established isolated OrchestratorError::Link/Layout boundary. Bytecode and PUSH-sequence callers continue to wrap compile errors in OrchestratorError::Compile. The singleton bytecodes are compile-time constants. Keep the legacy singleton oracle available to the full-image comparison until removal is separately reviewed. Normal and Underflow must still yield 42+7 and 3+8. The standalone Opcode::assembly, Rv64Runner, public standalone fragment generators and executor need no routing changes.
5. **Stop for acceptance.** Run all gates after each switch. Retire legacy code only under subsequent authorization. Keep independently reviewable changes so either route can be restored independently.

## Reproduction and acceptance

Run from the repository root in a fresh Cargo target directory for the cutover gate.

```sh
export CARGO_TARGET_DIR=target/phase-2.4-validation
cargo fmt --all -- --check
cargo clippy --frozen --workspace --all-targets -- -D warnings
cargo test --frozen -p ferreth-compiler --lib
cargo test --frozen -p ferreth-compiler --test rv64_push1_program
cargo test --frozen -p ferreth-compiler --test rv64_final_image
cargo test --frozen -p ferreth-conformance --test rc_revm_conformance
cargo test --frozen -p ferreth-linking-spike
cargo test --frozen --workspace -- --list
cargo test --frozen --workspace --no-fail-fast
cargo test --frozen -p ferreth-compiler --lib capacity -- --ignored --nocapture --test-threads=1
cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1
cargo run --frozen -q -p ferreth-conformance -- bench guest --format json
cargo run --frozen -q -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json
```

Close LF-026 only when all four unchanged public 47-PUSH cases execute and pass. Check the first trampoline at entry+40 with displacement +2056 (EOF) or +2060 (STOP). Normal publishes 47 twos with final s1 0x801015e0. Seeded overflow preserves all 1,024 words and s1 0x80109000, code 1. Keep the passing 46-PUSH controls and assert private guard/publication and 470/471+7 vs 4+8 metrics. Require full discovery reconciliation, zero unexpected skips or failures, all three opt-in probes, unchanged benchmark JSON payloads and hashes, independent ABI checks under ADR-001, and all negative controls. Compare with the 2.3d inventory and account for each migrated or added test.

**Rollback criterion.** An unexplained difference in endpoint, error classification, role, reassembled bytes, range, stack, publication, counts, protected core or frozen-suite result stops the affected switch. Restore its previous delegation and keep the failing vector plus private evidence. A restored legacy compiler also restores the four known public failures. No automatic runtime fallback from a failed symbolic verification to a legacy artifact.

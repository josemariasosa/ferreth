# Phase 2.3a — Opcode-independent symbolic fragment contract

**Status:** Bounded implementation complete; stopped for maintainer review.  
**Branch and base:** `feat/symbolic-guest-composer`, `f23caec3d8cbd6040d0a2c73db8eed826c102b38`.  
**Scope:** Private PUSH1/STOP fragment contract and composer boundary only. No public routing, mixed-opcode composition, POP/DUP1/ADD/SUB generation, gas behavior, dependency, or ABI changes.

This report follows accepted [ADR-001](../../docs/adr/001-evm-guest-state-abi.md), uses proposed [ADR-002](../../docs/adr/002-cancun-evm-gas-metering.md) as context only, and records the reviewed [Phase 2.3 boundary](phase-2.3-boundary-review.md). Phase 2.2 remains the artifact/runtime reference for the symbolic PUSH1/STOP candidate.

## 1. Previous versus revised ownership

| Responsibility | Phase 2.2 | Phase 2.3a |
|---|---|---|
| Fragment representation | `SymbolicFragment::Push1(u8)` / `Stop`; the composer chose how each variant emitted source. | A generated fragment carries source, an entry/ownership symbol, typed external declarations, local transfer declarations, and the verifier-only fixed layout contract. |
| Opcode emission | The composer matched fragment variants and called the PUSH1 emitter or emitted STOP itself. | The PUSH1 generator returns a complete fragment contract. The STOP frontend adapter returns its source and Normal declaration. |
| Publisher inventory | Hardcoded Normal + Overflow for every candidate, including empty input and STOP-only input. | Mandatory Normal plus the deduplicated roles actually present in external declarations, ordered deterministically. |
| Transfer binding | Fixed-layout verifier sites represented the intent; no fragment-owned transfer declarations existed. | Each fragment declares word-indexed external and local sites; the final-image verifier resolves symbols, independently decodes bytes, and compares actual transfers. |
| Composition | Source emission required an opcode match. | The core concatenates source, derives roles, appends publisher source, assembles once, resolves final labels, verifies, and constructs the artifact without matching opcodes. |

Public compile routes in [compiler.rs](../../crates/ferreth-compiler/src/compiler.rs) remain on the legacy implementation. The symbolic module's visibility is only `pub(crate)` so the private fragment type can be returned by the existing PUSH1 generator; it is not a public API change.

## 2. Final private Rust API

The private contract is in [symbolic.rs](../../crates/ferreth-compiler/src/compiler/symbolic.rs):

```rust
enum PublisherRole { Normal, Overflow }

struct ExternalDeclaration {
    site_word: u64,
    role: PublisherRole,
    kind: Transfer,
}

struct LocalDeclaration {
    site_word: u64,
    target_symbol: String,
    kind: Transfer,
}

struct SymbolicFragment {
    source: String,
    entry_symbol: String,
    external_references: Vec<ExternalDeclaration>,
    local_transfers: Vec<LocalDeclaration>,
    fixed_layout: Fragment,
}
```

`Transfer` is a concrete typed `Jal { rd }` or `Branch { funct3, rs1, rs2 }` in [verify.rs](../../crates/ferreth-compiler/src/rv64/verify.rs). Every site index is a zero-based 32-bit instruction word offset from `entry_symbol`. The entry symbol is the fragment ownership key; the generator's deterministic occurrence index namespaces all of its local names. Transfer-site labels are not added solely for bookkeeping. Local target labels are emitted only where the actual source needs them.

`fixed_layout` is not opcode state or a second lowering representation. It selects the existing Phase 2.1 independent fixed-layout completeness contract, which is deliberately retained for PUSH1 because declaration-driven checks alone cannot prove that a PUSH1 body omitted no required instruction or transfer.

The contract stores no EVM stack state, instruction semantics, register allocation, CFG, or general relocation objects. There are exactly two exceptional publisher roles, Normal and Overflow; OOG and underflow are not introduced.

## 3. Why `compose` is opcode-independent

`compose` does not match EVM opcodes or fragment kinds, inspect instruction strings, patch bytes, calculate displacements, or select exceptional destinations. Its steps are:

1. Reject duplicate fragment entry/ownership symbols.
2. Concatenate each already-generated `source` in sequence.
3. Derive and order publisher roles from declarations plus mandatory Normal.
4. Append publisher source for those roles and assemble once with pinned `asm-rs 0.2.0`.
5. Resolve publisher PCs and declared local targets from the final assembler label map.
6. Decode and verify the actual transfer words, then run the independent fixed-layout verifier.
7. Construct `CompiledRv64Program` only after all checks pass.

Adding another opcode generator that uses Normal/Overflow therefore requires no change to the core composition loop or publisher inventory algorithm. This phase intentionally does not add another generator.

## 4. Publisher inventory derivation

The inventory starts with mandatory Normal, adds all roles referenced by external declarations, sorts by the enum's stable role order (Normal before Overflow), and deduplicates. Multiple external sites may declare the same role without adding duplicate publishers. STOP declares Normal; each PUSH1 declares Overflow.

This means an empty fragment list and STOP-only input have no Overflow declaration and now emit only Normal. The maintainer confirmed this as the narrow intentional exception to Phase 2.2 exact inventory/PC parity. Each loses one unused 32-byte Overflow publisher; runtime/ABI behavior is unchanged. PUSH-bearing candidates retain the Phase 2.2 Normal/Overflow inventory and PCs.

## 5. External and local declaration semantics

- An external declaration identifies the transfer word relative to its owner, the intended typed publisher role, and the required transfer kind and fields. PUSH1 declares word 10 as `jal x0` to Overflow. STOP declares word 0 as `jal x0` to Normal.
- A local declaration identifies the instruction word, its kind/fields, and the target symbol. PUSH1 declares `bgeu s1,t0` at word 2 to its overflow trampoline and `jal x0` at word 9 to its continuation.
- The final label map supplies addresses only. The verifier independently reads little-endian machine words, decodes JAL/B-type fields and displacements, and compares actual destinations with the declaration-resolved target.
- The scan covers every fragment word before the Normal publisher. Undeclared, missing, duplicated, misaligned, out-of-bounds, or field/destination-mismatched transfers fail. The fixed-layout gate additionally rejects changed widths, unexpected branch relaxation, and any transfer inventory inconsistent with the known PUSH1/STOP layouts.

The assembler and declarations cannot certify one another: the independent PUSH1 fixed-layout check still requires the source-derived stack-limit branch, continuation, trampoline, exact widths, destination roles, and complete publisher bodies. Publisher ABI bodies remain checked as final machine words, including status-last stores and sentinel JALR.

## 6. Independent completeness verification

The existing Phase 2.1 verifier was minimally adapted to accept the derived symbolic publisher inventory (Normal-only or Normal + Overflow). The new declaration scan is in that verifier module; it adds no second verifier framework.

Verification combines:

- `validate_generated_code` for the fixed entry, nonempty image, four-byte length/alignment, and executable bound before the reserved exit slot.
- Symbol resolution from final assembler labels, without using relocation records as proof.
- Independent byte decoding for each declared transfer plus a scan for every unexpected JAL, branch, JALR, or non-base-width instruction in the fragment area.
- The prior fixed PUSH1/STOP completeness contracts and exact publisher ABI body checks.
- Exact final image consumption, publisher PC ordering, and in-image aligned transfer targets.

If a later fragment cannot yet be completely verified from generic declarations, it must retain an independently established fixed-layout gate until that completeness is justified. This phase does not weaken the existing PUSH1 checks.

## 7. Negative test evidence

The symbolic unit tests include:

- Missing external declaration and duplicate declarations.
- An encoded transfer whose declaration binds it to the wrong semantic role.
- Unexpected extra JAL and branch instructions.
- Duplicate fragment ownership/local-label collision.
- Corrupted JAL target encoding.
- Missing required publisher inventory entry.
- Misaligned declared site and publisher start outside executable bytes.
- Existing fixed-layout mutations for wrong STOP/Overflow role, wrong continuation, altered PUSH1 branch, and missing publisher label.

The retained verifier tests additionally reject wrong `rd`/branch fields, wrong local anchors/roles, signed JAL corruption, halfword targets, incomplete/duplicate/unordered declarations, unexpected JALR, non-32-bit widths, assembler branch relaxation, invalid site/target bounds, and trailing image bytes. All focused symbolic and verifier tests passed.

## 8. Candidate artifact and runtime parity

For any input containing PUSH1, source generation is unchanged from the Phase 2.2 candidate: the same fragment labels and instruction text are emitted, and the same publisher source is appended in Normal/Overflow order. Tests require the returned assembly to reassemble to the exact returned machine bytes. Short artifacts and public legacy-prefix comparisons, literal long-layout PCs/words, ABI publication traces, and dynamic execution observations all pass.

The only candidate artifact delta is the confirmed removal of unused Overflow when there are no PUSH1 transfers:

| Input | Phase 2.2 candidate | Phase 2.3a | Delta |
|---|---|---|---|
| Empty/EOF | 345 source bytes, 2 labels, 60 image bytes; Normal `0x80000000`, Overflow `0x8000001c` | 156 source bytes, 1 label, 28 image bytes; Normal `0x80000000` | −189 source bytes, −1 label, −32 image bytes; no Overflow PC |
| STOP only | 383 source bytes, 3 labels, 64 image bytes; Normal `0x80000004`, Overflow `0x80000020` | 194 source bytes, 2 labels, 32 image bytes; Normal `0x80000004` | −189 source bytes, −1 label, −32 image bytes; no Overflow PC |

The separate legacy explicit-STOP layout still has its known unreachable underflow publisher. The symbolic candidate remains smaller than that legacy image; Phase 2.3a removes only the candidate's now-undeclared Overflow publisher in the no-PUSH case.

For PUSH-bearing inputs, exact inventory, publisher PCs, static size, and execution metrics remain at Phase 2.2 values:

| Input | Bytes | Normal / Overflow PC | Successful RV64 body + publisher | Full-stack failure |
|---|---:|---|---:|---:|
| 1 PUSH1 | 104 | `0x8000002c` / `0x80000048` | 10 + 7 | 4 + 8 |
| 2 PUSH1 | 148 | `0x80000058` / `0x80000074` | 20 + 7 | 4 + 8 |
| 5 PUSH1 | 280 | `0x800000dc` / `0x800000f8` | 50 + 7 | 4 + 8 |
| 46 PUSH1, EOF | 2,084 | `0x800007e8` / `0x80000804` | 460 + 7 | 4 + 8 |
| 46 PUSH1, STOP | 2,088 | `0x800007ec` / `0x80000808` | 461 + 7 | 4 + 8 |
| 47 PUSH1, EOF | 2,128 | `0x80000814` / `0x80000830` | 470 + 7 | 4 + 8 |
| 47 PUSH1, STOP | 2,132 | `0x80000818` / `0x80000834` | 471 + 7 | 4 + 8 |

The 47-PUSH first Overflow JAL remains `0x0090006f` (+2056) for EOF and `0x00d0006f` (+2060) for STOP. The symbolic candidate executes normally with 47 twos and `s1=0x801015e0`; on a full seeded stack it publishes status GuestException, code 1, preserves all 1,024 words, and publishes `s1=0x80109000`. The regression observations also check the live `s0/s1`, native `sp`, stack guard, ABI metadata publication trace, and dynamic RV64 counts.

The candidate tests cover empty, one/two/five PUSH1, implicit EOF, explicit STOP, truncated PUSH1 zero padding, first STOP with unreachable trailing bytes, 46/47 EOF and STOP, and a 47-PUSH overflow execution. Legacy comparisons retain exact machine-code prefix parity after excluding only unreachable trailing publishers.

## 9. Capacity and label-count comparison

The ignored boundary test was explicitly rerun against the generated fragment contract with default `asm-rs` limits. It uses no extra transfer-site labels: PUSH1 remains three labels per occurrence; publisher and STOP labels remain unchanged. All measured source sizes, label counts, assembled sizes, and Ferreth layout outcomes match Phase 2.2:

| PUSH1s | STOP | Source bytes | Labels | Assembler | Image bytes | Ferreth layout |
|---:|---|---:|---:|---|---:|---|
| 23,829 | No | 7,713,049 | 71,489 | Success | 1,048,536 | Accepted |
| 23,830 | No | 7,713,375 | 71,492 | Success | 1,048,580 | Reject exit-slot overlap |
| 23,829 | Yes | 7,713,087 | 71,490 | Success | 1,048,540 | Accepted |
| 23,830 | Yes | 7,713,413 | 71,493 | Success | 1,048,584 | Reject exit-slot overlap |

No assembler resource limit was changed or silently raised. Ferreth's executable ceiling still binds before the 100,000-label default. The empty/STOP-only source delta is the omitted publisher described above; the measured PUSH boundary source sizes are unchanged.

## 10. Workspace counts and remaining LF-026 failures

| Verification | Result |
|---|---|
| Phase 2.3a symbolic tests | 7 passed, 1 ignored; boundary rerun 1 passed |
| Phase 1 research package | 22 passed, 1 ignored |
| Production compiler package | 144 passed, 4 failed, 1 ignored |
| Conformance package | 15 passed |
| Frozen workspace discovery | Exit 0; 187 discovered test names |
| Frozen workspace execution | 181 passed, 4 failed, 2 ignored; 0 measured/filtered |
| Formatting | `cargo fmt --all -- --check` passed |
| Strict Clippy | `cargo clippy --frozen --workspace --all-targets -- -D warnings` passed |

The only four workspace failures are the unchanged LF-026 regressions:

- `repeated_push1_47_eof_completes_normally`
- `repeated_push1_47_eof_full_stack_publishes_overflow`
- `repeated_push1_47_stop_completes_normally`
- `repeated_push1_47_stop_full_stack_publishes_overflow`

All four remain discovered with unchanged names and expectations. They still fail on the public legacy route during assembly with `ImmediateOverflow`, value +2056 (EOF) or +2060 (STOP), reported assembler bounds −2048 through +2047 at line 12, column 5. No new workspace failure appeared. The 47-PUSH candidate path itself passes its symbolic runtime tests; public routing was intentionally not switched.

## 11. Changed-file inventory and scope review

| File | Scoped change |
|---|---|
| [compiler.rs](../../crates/ferreth-compiler/src/compiler.rs) | Makes the still-private symbolic module crate-visible for the PUSH1 fragment return type; does not change public routing. |
| [symbolic.rs](../../crates/ferreth-compiler/src/compiler/symbolic.rs) | Replaces the opcode-shaped enum with the fragment contract and opcode-independent composition/inventory flow. |
| [push.rs](../../crates/ferreth-compiler/src/rv64/opcodes/push.rs) | PUSH1 generator now emits its complete symbolic source and verification declarations. |
| [verify.rs](../../crates/ferreth-compiler/src/rv64/verify.rs) | Adds declaration-driven final-byte scan and the Normal-only symbolic publisher layout; retains fixed-layout completeness checks. |
| [symbolic/tests.rs](../../crates/ferreth-compiler/src/compiler/symbolic/tests.rs) | Adds declaration/inventory rejection cases and adjusts candidate parity assertions for the unused-publisher delta. |
| [README.md](README.md) | Adds the Phase 2.3a research index entry. |
| [ferreth-linking-architecture.md](ferreth-linking-architecture.md) | Records the bounded Phase 2.3a stop point in the roadmap. |
| This report | Records implementation and verification evidence. |

No manifest, lockfile, public compile route, compiler conformance test, executor, ABI, unrelated research evidence, or Phase 2.3 boundary-review content was changed. The pre-existing untracked `phase-2.3-boundary-review.md` was preserved. No branch operation, commit, or push was performed.

## 12. Phase 2.3a exit gate

**Met for this bounded chunk, with the confirmed no-PUSH publisher delta documented above.** The opcode-shaped fragment enum and hardcoded universal inventory are gone; fragment-owned declarations drive transfer verification and publisher roles; all PUSH1/STOP fixed-layout and ABI gates remain; capacity and test evidence are recorded; public routes and LF-026 controls remain unchanged.

Stop here for maintainer review. Do not begin POP/DUP1, ADD/SUB, mixed composition, public cutover, or legacy removal.

# Phase 2.3b — Symbolic POP and DUP1

**Status:** Bounded implementation complete; stopped for maintainer review.  
**Branch and base:** `feat/symbolic-guest-composer`, `a726583aa97cf1e6702d4d8d942e1f70ee39ff68`.  
**Scope:** Private symbolic POP and DUP1 fragments using the Phase 2.3a contract. No ADD/SUB, GAS, JUMP, public cutover, legacy removal, or public-route migration.

This report follows accepted ADR-001 and proposed ADR-002 as context only.

## 1. Executive summary

Phase 2.3b adds private symbolic POP and DUP1 lowering while preserving the
opcode-independent `compose(&[SymbolicFragment])` path. POP uses a 4-word
fixed layout and DUP1 uses a 16-word fixed layout. The publisher inventory
now derives Normal, Underflow, and Overflow from fragment declarations.

Exit gate: met for the bounded POP/DUP1 milestone. Public compilation remains
on legacy routes and the four LF-026 public PUSH1 regressions remain unchanged.

## 2. Initial worktree and revision

Initial branch status was `## feat/symbolic-guest-composer` with no modified
or untracked entries shown in `git status --short --branch`. Initial revision: `a726583aa97cf1e6702d4d8d942e1f70ee39ff68`.

No branch switch, commit, push, reset, clean, merge, or public migration was
performed.

## 3. Changed-file inventory

| File | Scoped change |
|---|---|
| `crates/ferreth-compiler/src/compiler/symbolic.rs` | Added private POP/DUP1 frontend lowering and Underflow/three-role inventory support. `compose` remains fragment-independent. |
| `crates/ferreth-compiler/src/rv64/opcodes/pop.rs` | Added symbolic 4-word POP source and typed local/external transfer declarations. |
| `crates/ferreth-compiler/src/rv64/opcodes/dup.rs` | Added symbolic 16-word DUP1 source and typed underflow/overflow/local declarations. |
| `crates/ferreth-compiler/src/rv64/verify.rs` | Added symbolic Normal/Underflow and Normal/Underflow/Overflow layout modes while retaining the fixed POP/DUP1 completeness contracts. |
| `crates/ferreth-compiler/src/compiler/symbolic/tests.rs` | Added POP/DUP1 inventory, success, exceptional, mixed, negative, and opt-in capacity evidence. |
| `research/guest-linking/phase-2.3b-pop-dup1.md` | This report. |
| `research/guest-linking/README.md` | Added the Phase 2.3b index entry. |

## 4. Opcode independence

`compose` still concatenates generated fragment sources, derives and orders
publisher roles from declarations, assembles once, resolves label addresses,
scans and verifies transfers, and runs fixed-layout verification. It contains
no POP or DUP1 match and performs no displacement calculation or machine-word
patching.

The private frontend dispatches decoded POP and DUP1 operations to generators.
This is the permitted private lowering boundary; it does not change
`compose`.

## 5. Generator and fragment contracts

### POP

POP retains the proven 4-word body:

1. `bgeu s0,s1` checks for empty-stack underflow.
2. `addi s1,s1,-32` removes one word on success.
3. `jal zero,<continuation>` transfers to the next fragment or Normal.
4. Local underflow trampoline transfers to the shared Underflow publisher.

The branch and the underflow JAL are declared; the success JAL is declared
locally for a fragment continuation or externally as Normal at EOF. The
representation needs one entry label and one underflow local label per
occurrence. It does not use transfer-site labels.

### DUP1

DUP1 retains the proven 16-word body:

1. Detect underflow before any stack read.
2. Derive the one-past-end stack limit from `s0` and detect overflow before
   any stack write.
3. Load all four limbs and store all four limbs.
4. Increment `s1` after the stores.
5. Transfer success to the next fragment or Normal.
6. Transfer underflow and overflow to their declared shared publishers.

The two local branch targets and the success JAL are declared, plus external
Underflow and Overflow destinations. Repeated occurrences use unique
`guest_dup1_*`, `dup1_underflow_*`, and `dup1_overflow_*` names.

## 6. Publisher inventory

Inventory is mandatory Normal plus roles referenced by external declarations,
deduplicated and ordered as Normal, Underflow, Overflow.

| Input | Inventory |
|---|---|
| Empty or STOP-only | Normal |
| PUSH1-only | Normal, Overflow |
| POP-only | Normal, Underflow |
| DUP1-only | Normal, Underflow, Overflow |
| Mixed PUSH1/POP/DUP1 | Union of required roles |

The existing shared compiler publisher source supplies all bodies.

## 7. Verifier and adversarial evidence

The existing declaration verifier now resolves external and local targets
from the final assembler label map and independently decodes JAL/B-type bytes.
It checks transfer register fields, signed offsets, anchors, target PCs,
publisher bodies and image consumption.

The fixed POP/DUP1 completeness gate remains independent of declarations:

- POP: 4 words, underflow branch at 0, success JAL at 2, underflow JAL at 3.
- DUP1: 16 words, underflow branch at 0, overflow branch at 3, success JAL
  at 13, underflow JAL at 14, overflow JAL at 15.

The existing final-image suite and mixed public image checks reject swapped
DUP1 underflow/overflow destinations and wrong local anchors. Focused symbolic
tests also cover missing, duplicate, misaligned and corrupted declarations,
wrong roles, extra transfers, duplicate ownership, and missing publishers.

## 8. Success and exceptional runtime results

The private success specimens POP and DUP1 were executed through rvemu:

- POP `[2] -> []`: Normal, 3 body + 7 publisher RV64 instructions.
- DUP1 `[2] -> [2,2]`: Normal, 14 body + 7 publisher instructions.
- DUP1 -> POP `[2] -> [2]`: Normal, 17 body + 7 publisher instructions.
- Mixed PUSH1/POP/DUP1 composition was assembled and verified.

Independent private exceptional publication was observed:

- POP on empty stack reaches Underflow, status `GuestException`, code 2,
  final `s1` at the stack base, and empty live stack.
- DUP1 on empty stack reaches Underflow, status `GuestException`, code 2,
  and unchanged empty stack.
- DUP1 on the full 1,024-word stack reaches Overflow, status
  `GuestException`, code 1, final `s1` at `EVM_STACK_REGION.end()`, and all
  1,024 original words preserved.

Exact candidate/legacy machine-byte parity is unchanged for the existing
PUSH1/STOP tests. Public routing was not switched, so POP/DUP1 parity is
documented only for the private candidate and independent legacy semantics
already established by the public mixed linker.

## 9. POP/DUP1 label-density and capacity

The opt-in capacity probe observed:

| Fragments | Labels per fragment | Labels | Source bytes | Assembled bytes | Ferreth/asm result |
|---:|---:|---:|---:|---:|---|
| 33,332 POP | 2 | 66,666 | 5,555,683 | 533,372 | Accepted |
| 33,333 POP | 2 | 66,668 | 5,555,851 | 533,388 | Accepted |
| 65,535 POP | 2 | not assembled | 10,965,787 | not assembled | asm-rs 100,000-label resource limit |
| 65,536 POP | 2 | not assembled | 10,965,955 | not assembled | asm-rs 100,000-label resource limit |
| 33,331 DUP1 | 3 | not assembled | 14,832,832 | not assembled | BranchOutOfRange to Underflow |
| 33,332 DUP1 | 3 | not assembled | 14,832,279 | not assembled | BranchOutOfRange to Underflow |
| 33,333 DUP1 | 3 | not assembled | 14,833,726 | not assembled | asm-rs 100,000-label resource limit |

POP safely removes transfer-site labels; it uses 2 labels per occurrence.
The measured 33,332/33,333 cases accept the image and are below executable
bounds. Larger POP probes meet the 100,000-label assembler limit before
executable layout acceptance.

DUP1 uses 3 labels per occurrence (entry, underflow, overflow). Its distant
Underflow branch exceeds the `asm-rs` 1,048,574-byte branch span at much
smaller counts, then the 100,000-label limit binds at 33,333. No limits or
code bounds were changed.

## 10. PUSH1/STOP regression results

The Phase 2.3a PUSH1/STOP suite remained green:

- Empty/STOP-only Normal-only inventory.
- PUSH1 Normal/Overflow inventory.
- Short PUSH1, truncated PUSH1, explicit STOP, and 46-PUSH controls.
- Existing fixed-layout and publisher ABI checks.

Formatting and strict workspace Clippy pass.

The four unchanged LF-026 public legacy failures remain:

- `repeated_push1_47_eof_completes_normally`
- `repeated_push1_47_eof_full_stack_publishes_overflow`
- `repeated_push1_47_stop_completes_normally`
- `repeated_push1_47_stop_full_stack_publishes_overflow`

They fail during legacy public display-assembly reassembly and are not
classified as successful tests.

## 11. Workspace totals

Final frozen discovery: 190 test names.

Final frozen complete workspace execution: 183 passed, 4 failed, 3 ignored,
0 measured, 0 filtered, exit 101.

Verification commands and outcomes:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --frozen --workspace --all-targets -- -D warnings`: passed.
- `cargo test --frozen -p ferreth-linking-spike`: passed.
- `cargo test --frozen --workspace -- --list`: 190 names discovered.
- `cargo test --frozen --workspace --no-fail-fast`: 183 passed, 4 failed,
  3 ignored.

The three ignored tests are the existing/retained capacity experiments,
including the bounded POP/DUP1 probe.

## 12. Remaining risks and next action

Remaining risks:

- DUP1 large sequences can bind at branch range or label density before the
  executable bound.
- POP/DUP1 private exceptional candidates need broader boundary-stack and
  mixed-sequence differential coverage.
- Public mixed-route machine layouts and inventory remain intentionally
  separate from private symbolic layouts.
- Dynamic instruction counts and exact parity do not alter the accepted ABI.

No public cutover, ADD/SUB, GAS, JUMP, ADR-002 implementation, or legacy
removal is authorized by this milestone.

**Phase 2.3b exit gate:** met for maintainer review. Stop here.

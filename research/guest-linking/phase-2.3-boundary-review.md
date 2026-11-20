# Phase 2.3 — Architecture Boundary Review

**Scope:** Repository and source-level review only. No production code, tests,
dependencies, branches, or gas implementation were changed.

**Current revision reviewed:**

- Branch: `feat/symbolic-guest-composer`
- Revision: `f23caec3d8cbd6040d0a2c73db8eed826c102b38`
- Worktree: clean; no staged, unstaged, untracked, stash, or additional
  working-tree changes were recorded.

This report distinguishes observed facts, source-derived findings, and
architectural recommendations.

## 1. Executive summary

**Decision:** The current implementation does not yet safely support mixed
opcodes without adjusting a small part of its boundary.

The private symbolic composer has the right outer shape: one source, one
`asm-rs 0.2.0` assembly pass, labels resolved from the final image, and
independent machine-code verification. It does not currently carry enough
general transfer intent to let POP, DUP1, ADD, SUB, and future GAS or OOG
become ordinary fragment emitters.

The smallest adjustment is **Option B**: keep `SymbolicFragment` as an
opcode-emitted fragment object, but add a minimal typed transfer declaration
that names external roles, local symbols, and required control-transfer sites.
Keep fixed fragment width and fixed publisher layout checking in the verifier
as temporary layout fixtures, not as a set of opcode semantics in the
composer. Do not introduce a general IR or relocation framework.

The legacy mixed composer is also coupled to opcode-specific assembly patching
and manual JAL encoding. It must remain as a compatibility route; it should not
be expanded or reused as the composition algorithm.

## 2. Current ownership table

| Responsibility | Candidate symbolic path | Legacy production route |
|---|---|---|
| EVM decoding/lowering | `symbolic::compile` decodes with `decode_detailed` and lowers PUSH1/STOP [compiler/symbolic.rs:26-46] | `compile_evm_bytecode` lowers supported operations [compiler.rs:119-163] |
| Symbolic source creation | `symbolic::emit` matches the private fragment enum [compiler/symbolic.rs:55-70] | `compile_supported_program` and `compile_push1_program` construct assembly and bytes by opcode route [compiler.rs:177-410,571-645] |
| Opcode RV64 semantics | The private PUSH1 emitter in `rv64/opcodes/push.rs` owns body and local labels [push.rs:35-63] | Each legacy generator owns opcode semantics [rv64/opcodes/*.rs] |
| Local label naming | PUSH1 emitter names entry, overflow, and continuation with its occurrence index [push.rs:35-63] | Linker adds program indexes or fixed labels and manually patches text/bytes [compiler.rs:246-344] |
| External destination declaration | Symbolic emitter receives `Publisher::Overflow.label()` directly [symbolic.rs:55-65]; no separate declaration record exists | The operation-specific match supplies overflow/underflow/normal PCs [compiler.rs:246-344] |
| Publisher inventory | Hardcoded `PUBLISHERS = [Normal, Overflow]` [symbolic.rs:18] | Mixed or PUSH-only layout is selected by compiler route [compiler.rs:149-163,403-407,630-635] |
| Publisher generation | `Publisher::assembly` emits one ABI publisher [publisher.rs:24-49] | Shared `Publisher::assembly` in mixed routes; isolated arithmetic has separate orchestrator assembly [publisher.rs, orchestrator.rs:405-483] |
| Complete guest assembly | `symbolic::compose` invokes `Assembler::emit` once and `finish` once [symbolic.rs:49-79] | Legacy linker manually concatenates generator/publisher machine code and separately assembles display source [compiler.rs:330-402] |
| Final publisher PCs | `label_address` resolves the two labels after assembly [symbolic.rs:88-96] | PCs are calculated from fragment/publisher widths [compiler.rs:568-576, orchestrator.rs:434-440] |
| Independent transfer verification | `verify_final_image` decodes final JAL/B-type words and fixed publisher bodies [rv64/verify.rs:36-64,262-329] | The same fixed-layout verifier is called after manual linking [compiler.rs:403-410,630-635] |
| Transfer completeness | Fixed-width fragment sequence and fixed `Fragment` variants declare sites [rv64/verify.rs:19-29,90-149] | The caller derives fixed fragment sequence and layout; callers do not independently declare per-site semantic roles [compiler.rs:403-407] |
| `CompiledRv64Program` construction | `verified_program` constructs fields after bounds, label, and image verification [symbolic.rs:113-121] | Each legacy route constructs the same artifact after linking and verification [compiler.rs:410-414,637-645] |

## 3. Observed facts

### 3.1 The private candidate boundary

1. `SymbolicFragment` has only `Push1(u8)` and `Stop`
   [compiler/symbolic.rs:20-24]. It is not presently an EVM instruction enum,
   but it is an opcode-shaped enum and requires a composer match before new
   opcode emitters can be added.
2. `emit` performs an opcode-specific match and independently chooses emitter
   behavior [compiler/symbolic.rs:55-66]. This is a real warning sign.
3. The composer hardcodes the `Normal, Overflow` inventory
   [compiler/symbolic.rs:18]. Its verifier call also hardcodes
   `PublisherLayout::PushOnly` [symbolic.rs:105-110].
4. PUSH1 owns local names and external overflow use in `emit_symbolic_push1`
   [rv64/opcodes/push.rs:35-63]. It does not calculate the publisher PC.
5. Assembly uses one public `Assembler::emit`/`finish` pass
   [symbolic.rs:73-79]. Symbol resolution and label addresses are delegated
   to `asm-rs`; there is no manual displacement calculation in this path.
6. The independent verifier decodes final JAL and B-type instructions,
   checks fields and targets, scans for unexpected control transfers, and
   consumes exact publisher layouts [rv64/verify.rs:260-329]. It is
   independent of assembler relocation records.
7. The candidate has no separate compile-time transfer declaration. The
   fixed-width fragment enum and verifier's opcode match are effectively the
   declaration [rv64/verify.rs:19-29,90-149].
8. `CompiledRv64Program` is constructed only after validation
   [symbolic.rs:113-121].

### 3.2 The legacy boundary

1. The public mixed route uses a `CompiledOperation` match and manually adds
   opcode-specific assembly/bytes [compiler.rs:177-344].
2. It manually encodes JAL displacements using `encode_jal_zero`
   [compiler.rs:246-344] and legacy `relative_jal_offset`/text patch helpers.
3. The isolated arithmetic route also replaces a source snippet and patches a
   machine word [orchestrator.rs:405-483].
4. The legacy route reassembles display assembly and compares it with machine
   bytes [compiler.rs:382-402]. Reassembly is an artifact presentation check,
   not independent intent verification.
5. The legacy route retains POP/DUP1/ADD/SUB support and fixed verifier layouts
   [compiler.rs:181-236,403-410].

### 3.3 Capacity observations

The pinned assembler defaults are 100,000 defined labels and Ferreth's
executable bound is 1,048,572 bytes [phase-2.0-readiness.md:38-82].
PUSH1's tested layout uses 44 bytes and three labels per occurrence, plus
publishers and STOP [phase-2.2-symbolic-guest-composer.md:135-139].
The POP body is 16 bytes, not 20, and the Phase 1 POP representation used
three labels per occurrence [phase-2.0-readiness.md:68-82].
The 33,332/33,333 POP pair reaches the label limit before Ferreth's size
bound [phase-2.0-readiness.md:68-82].

The mixed symbolic fixture used distinct names for 52 or 53 labels and checked
54 or 55 control transfers [1.6-mixed-symbolic-guest.md:61-66].

### 3.4 Independent verification evidence

Phase 1.3 established literal JAL/B-type decoding and negative fixtures,
including out-of-range, misalignment, wrong register fields, wrong targets,
assembler relaxation, missing declarations, swapped publishers, and wrong
arithmetic calls and returns [phase-2.1-final-image-verification.md:98-103].
The verifier is therefore capable of rejecting disagreement with a declared
layout, but the declaration is currently duplicated and fixed-layout rather
than fragment-produced.

## 4. POP/DUP1/ADD/SUB extensibility walkthrough

### 4.1 POP

The POP generator can emit:

- a local underflow destination;
- an external transfer from the underflow trampoline to the Underflow publisher;
- a local continuation branch/jump.

It should emit a fragment with local and external symbolic references. Under
the proposed contract, the composer should not contain a `SymbolicFragment::Pop`
match or a POP-specific publisher branch.

**Source-derived result:** POP is 4 instructions, with underflow and
continuation `jal` targets [rv64/opcodes/pop.rs:26-42]. The verifier fixed
layout currently models its three required sites [rv64/verify.rs:114-121].

### 4.2 DUP1

DUP1 has underflow, overflow, and normal continuation. It can represent all
three uniformly as external-role declarations plus continuation:

- local underflow label → `PublisherRole::Underflow`;
- local overflow label → `PublisherRole::Overflow`;
- normal fallthrough → the next fragment entry or `PublisherRole::Normal`.

The two underflow/overflow transfers can currently be confused semantically
even if bytes and PCs are valid. Verification must compare the independently
decoded destination role and, for the fixed contract, literal branch operands
and local destinations. `PublisherLayout::Mixed` already names all three
roles, but the fragment body does not declare which role belongs to each
site.

**Source-derived result:** DUP1 has 16 instructions and two exceptional
trampolines [rv64/opcodes/dup.rs:27-51]; fixed verification currently checks
five transfer sites [rv64/verify.rs:123-132].

### 4.3 ADD and SUB

ADD/SUB require underflow, an embedded arithmetic core call, a return to the
`s1 -= 32` instruction, normal continuation, and underflow external transfer.
The fragment emitter owns these names and local call/return relationships.
The composer should only place the fragment and resolve declared symbols.

Local names must include occurrence ordinals, for example
`add_0_underflow`, `evm_add_0_leaf`, and `add_0_return`, plus a disjoint
leaf/return namespace for repeated occurrences.

The verifier must decode the wrapper call and return anchor as generic JAL
and JALR fixed-instruction sites. It should not know ADD versus SUB or core
structure. It can know a generic arithmetic fragment layout contract supplied
from the generator for temporary fixed layouts.

**Source-derived result:** ADD and SUB are 43-word fixed layouts
[phase-2.1-final-image-verification.md:57]. The wrapped leaf call and
continuation are already represented in `GeneratedRv64LinkMetadata`
[rv64/codegen.rs:24-33].

### 4.4 Future GAS and OOG

A GAS generator may declare an external OOG role and local continuation.
This is legitimate if the publisher inventory gains a semantic `OOG`
publisher. It becomes an opcode-specific linker branch only if the composer
contains a `GAS`/`OOG` match or special offset algorithm.

Under ADR-002, a gas cost is an opcode-side semantic preflight. The linking
contract should not decide gas; it should only permit the generator to declare
an external role and verify that the OOG publisher was emitted and reached
[docs/adr/002-cancun-evm-gas-metering.md:3-6,63].

## 5. Architectural warning signs

| Finding | Classification | Evidence |
|---|---|---|
| `match` in symbolic emission by fragment variant | **Confirmed architectural coupling** | The composer must grow an opcode branch for every fragment [compiler/symbolic.rs:55-66]. |
| `SymbolicFragment` variants duplicate EVM opcode subset | **Potential risk; will become coupling** | Currently only PUSH1/STOP; mixed opcodes would duplicate the existing EVM enum [compiler/symbolic.rs:20-24]. |
| Hardcoded PUSH1 publisher inventory | **Confirmed architectural coupling** | `[Normal, Overflow]` is private to Phase 2.2 [compiler/symbolic.rs:18]. |
| Legacy source-string replacement and manual JAL patching | **Confirmed coupling in legacy route** | The legacy route is an explicit compatibility implementation, not the symbolic candidate [compiler.rs:246-344; orchestrator.rs:443-472]. |
| Fixed verifier `Fragment` and `PublisherLayout` enums | **Acceptable temporary Phase 2.2 limitation** | They encode required layouts and publishers but are the same fixed-layout mechanism used by Phase 2.1 [rv64/verify.rs:19-29,90-149]. |
| Fixed 11/4/16/43 layout and exact transfer indices | **Potential risk requiring evidence** | Appropriate as a temporary verified contract; it must reject relaxation until layout metadata changes [rv64/verify.rs:90-149]. |
| Verification expectations derived from assembler result? | **Not a problem currently** | Verification reads machine bytes and independently expects PCs and literal fields [rv64/verify.rs:260-329]. |
| Hidden undeclared external transfer | **Potential risk** | The composer presently declares verifier intent only through fixed `Fragment`; fragment emission does not return separate declarations [symbolic.rs:99-110]. |
| Duplicate assembler ownership and public APIs | **Acceptable during cutover** | Public/legacy routes remain intentionally separate; only the internal candidate is private [symbolic.rs:7,25; compiler.rs:22-24]. |

## 6. SymbolicFragment evaluation

1. **Generic enough for existing supported opcodes?** Not for mixed opcodes.
   The private enum is sufficient for PUSH1/STOP only, but will become a second
   EVM instruction representation if extended directly.
2. **Minimum information?** It currently carries opcode payload and an internal
   `Push1` immediate. Minimum fragment information should instead be
   generated source plus local symbolic namespace plus transfer declarations.
3. **Missing information?** It does not carry external roles, local branch/call
   anchors, required fixed sites, or publisher inventory requirements.
4. **Local names?** Use deterministic occurrence ordinals in lowered instruction
   order. Reserve publisher and source namespaces. Emit disjoint entry,
   continuation, local branch, call, leaf, and return labels. Duplicate
   definitions must fail assembly.
5. **External references?** Use both symbolic roles and explicit declaration
   records. The publisher role is the semantic destination; the declaration
   records the site and expected transfer kind/fields.
6. **Publisher inventory?** The composer may derive inventory from a set
   union of external declarations. It must not inspect opcode-specific source
   patterns or decode EVM semantics. It may receive no declarations and still
   emit only Normal through continuation.
7. **Declaration completeness?** Independently compare the fragment-emitted
   declarations against final image scan results and required inventory.
   Reject missing, duplicated, unordered, undeclared, or extra JAL/JALR/B-type
   sites.
8. **asm-rs label limit?** Symbolic labels should be minimized. Use one entry
   label per fragment only when it is needed, while local branches/calls need
   stable names. Avoid separate continuation labels that alias the next entry
   without also identifying a required declaration. Repeated PUSH1 should not
   require one label per transfer plus per-PC metadata. A 16-byte POP currently
   may be represented with entry/underflow/trampoline names and a continuation
   alias, and should not use a permanent label for an unused continuation
   when it equals the next entry.
9. **IR risk?** The representation remains small if a fragment is only
   `(source, namespace, declarations)`. It becomes an IR if it reproduces EVM
   stack state, register allocation, basic-block CFG, or general relocations.

### Minimal Rust API sketch

```rust
enum PublisherRole {
    Normal,
    Overflow,
    Underflow,
    // OOG is added only after separate maintainer approval.
}

enum ExternalTransferKind {
    JalX0,
}

struct ExternalDeclaration {
    site_symbol: String,
    role: PublisherRole,
}

struct SymbolicFragment {
    source: String,
    local_namespace: String,
    external_references: Vec<ExternalDeclaration>,
}
```

This is a sketch for comparison only. The implementation should keep role
records and fixed final-layout records separate: the fragment declaration
describes intent, while the verifier declaration describes independently
expected final machine-code sites.

## 7. Independent verifier boundary

The Phase 2.1 verifier is reusable, but its fixed `Fragment` and
`PublisherLayout` contracts should be split from the composer.

The smallest verification contract is:

1. **Image contract:** base/alignment, complete 32-bit words, code bounds,
   and exact image consumption.
2. **External role contract:** every external declaration resolves to a
   publisher label; its final machine-code JAL/B-type site decodes to the
   independently computed publisher PC.
3. **Local contract:** each fragment supplies required local site indices,
   transfer kind/register fields, and exact local target anchors. Local branch
   relaxation remains rejected until separately implemented and tested.
4. **Call contract:** arithmetic calls require `jal ra` at the declared call
   site and a fixed `PC+4` update anchor/return target. The verifier must not
   identify the ADD/SUB core; it validates the declared local transfer and
   return.
5. **Publisher contract:** verify ordered publisher PCs and full literal ABI
   bodies independently of assembler relocation records.
6. **Completeness contract:** scan every fragment and publisher word for extra
   JAL/JALR/B-type control transfers, duplicate or out-of-range declarations,
   and inserted instructions.
7. **Inventory contract:** derive required publishers from fragment declarations
   plus continuity, but independently verify that emitted publisher labels and
   PCs match one and only one semantic role.

### Trust boundary

The assembler may be trusted to assemble syntactically valid source correctly,
but not to certify semantic intent. The composer may be trusted to place and
label fragments, but not to certify that declarations matched bytes. The
final verifier must read machine-code words, independently calculate PCs, and
reject semantically swapped destinations even when the assembler and composer
agree.

## 8. Label capacity and complexity

The proposed contract should not require three labels for every simple local
check. It needs labels for:

- actual source entry points;
- required external and local targets;
- arithmetic core call targets and return anchors;
- shared publishers.

For POP, the entry and underflow targets plus continuation alias should be
enough for the fixed 16-byte body if the continuation is always the next
entry or normal publisher. Requiring an entry and a continuation label plus
an underflow label for every POP is the unnecessary pattern currently
evidenced as three labels per occurrence.

DUP1 needs entry, underflow, overflow, and continuation anchors as required
by actual sites. Name growth remains linear and can be bounded by the
declared layout.

No assembler resource-limit increase is justified by this abstraction.
PUSH1 and POP evidence shows that label exhaustion can bind first. The
composer should reduce label density and verify within the existing defaults.

The symbolic design is simpler than the legacy linker only if it removes:

- opcode-specific machine patching;
- opcode-specific display source manipulation;
- per-route publisher layout calculation;
- reassembly as the only machine consistency check.

It is not simpler merely because it uses a string and `SymbolicFragment`
match. It should also preserve public routes and isolated arithmetic as
compatibility adapters during cutover.

## 9. Alternatives

| Option | Correctness | Extensibility | Complexity | Verification independence | Gas readiness |
|---|---|---|---|---|---|
| A. Extend 2.2 unchanged | Weak | Needs composer/opcode/verifier branches | Small first step, but new duplication | Partial fixed-layout checks remain | Special GAS/OOG branches likely |
| B. Minimal fragment/reference contract | Strong | New emitters declare external/local intent generically | Small API and focused verifier changes | Separate intent and bytes | Uniform new publisher roles |
| C. Structured symbolic IR | Strongest in principle | General CFG/relocations | Highest complexity and scope growth | Could be independent, but expensive | Generic framework possible |

**Recommendation:** Choose B. It satisfies the desired responsibility split
and provides a traceable declaration to the independent verifier without
generalizing into an IR.

## 10. Recommended decision

Adjust the private composer boundary before adding mixed opcodes.

The adjustment should:

1. replace the opcode-variant private enum with a fragment that carries
   source, namespace, external declarations, and optional fixed-layout
   verification metadata;
2. let opcode emitters declare external publisher roles rather than hardcoding
   publisher names into the emitter call;
3. derive publisher inventory by set union of declarations;
4. keep local label and local transfer ownership in opcode emitters;
5. keep publishing, assembly, label resolution, and artifact construction in
   the composer;
6. keep independent machine decoding and semantic publisher verification in
   the verifier;
7. remove fixed opcode semantics from the composer, not from the verifier,
   until a more general declaration can replace the temporary verifier table;
8. retain all legacy routes until symbolic parity and cutover review pass.

## 11. Acceptance criteria for beginning Phase 2.3 implementation

Phase 2.3 implementation may begin only when all of the following are
satisfied:

- [ ] A new opcode can be added without changing `compose`, publisher placement,
  or publisher inventory selection.
- [ ] An emitter may declare multiple external roles uniformly.
- [ ] The composer derives publisher inventory from declarations and cannot
  silently omit a required publisher.
- [ ] Local label uniqueness is deterministic and collision-free.
- [ ] The final verifier independently decodes JAL/JALR/B-type sites and
  compares them to declaration-derived expected targets.
- [ ] DUP1 underflow and overflow destinations are independently distinguishable
  and cannot be semantically swapped.
- [ ] ADD/SUB local call and return targets are declaration-checked and
  machine-decoded.
- [ ] Unexpected instruction expansion or undeclared control transfer is rejected.
- [ ] Publisher full bodies are checked independently of relocation records.
- [ ] POP label density has a measured representation below the 100,000-label
  default for the relevant conformance gate.
- [ ] Public legacy output and public APIs remain available during cutover.

## 12. Open questions requiring maintainer approval

1. Should `SymbolicFragment` be removed entirely in favor of a
   `SymbolicFragment { source, namespace, declarations }` structure, or should
   opcode payload remain as a typed Emitter result?
2. Is a temporary fixed-layout verifier table acceptable for POP/DUP1/ADD/SUB
   before a general declaration-driven verifier is implemented?
3. Which labels are intentionally aliases of continuation/next-entry PCs, and
   may the implementation use aliasing only if declarations remain unique?
4. Should the mixed publisher inventory become declaration-derived for all
   public routes, or retain a compatibility route with preselected inventories?
5. Does the project require OOG as a publisher role in the next implementation
   milestone, or is it a later ADR-002 activation?
6. When may legacy text replacement and manual JAL patching be removed?
   This requires positive and negative final-image parity evidence, not a source
   count.

## 13. Findings classification summary

### Confirmed architectural coupling

- Opcode-specific symbolic emission match.
- Hardcoded candidate publisher inventory and layout.
- Legacy manual JAL patching and text replacement.

### Potential risks requiring evidence

- Fixed fragment width and site indices if new emitters or relaxation are added.
- Missing separate external transfer declarations.
- Label growth from continuation and trampoline naming.
- Publisher inventory disagreement with an emitter declaration.

### Acceptable temporary Phase 2.2 limitations

- Two-fragment enum for PUSH1/STOP.
- Hardcoded Normal/Overflow PUSH-only layout.
- Fixed-width verifier tables for the already Phase 2.1-known fragments.
- Legacy route duplication during compatibility cutover.

### Not problems

- Opcode semantics in opcode generators.
- One assembler pass in the symbolic candidate.
- Independent machine decoding in the verifier.
- Publisher source shared at the compiler layer.
- Retaining legacy routes and public APIs until compatibility evidence passes.

# Ferreth — Human-Led Walk

**Independent architecture and correctness review after Phase 2**  
**Status:** Checkpoints A–G investigated, human exit gate pending · **Scope:** symbolic RV64 guest linking for the current six-opcode EVM subset

**Branch history** — Phase 2.4 cutover `3eb4a94`, Phase 2.5 cleanup `831ac85` on `feat/symbolic-guest-composer`.

**Read this file first when returning to Ferreth.**

> **Mission.** We built a compiler path that transforms supported EVM bytecode into an executable RV64 guest, then runs that guest and compares observable behavior against trusted expectations. The human-led walk is **not** another automated green-test gate. It is an attempt to understand the entire trust chain, challenge assumptions, distinguish independent evidence from self-consistency, and decide what must be stabilized before a first `v0.1.0` baseline.
>
> **Operating rule:** the human reviewer leads; assistants are copilots. Do not begin by asking an agent to audit the project or summarize what it thinks is correct. Read code and trace a specimen first, write down your own hypothesis, then use the assistant to test it.

---

## Session Handoff — 2026-11-21

The A–D handoff was parked at `df0d1f7`, immediately after the H-WALK-003 regression commit `ffe67f3`. The documentation consolidation checked a clean `feat/symbolic-guest-composer` worktree at `df0d1f7e08fdb0168a4fc2e6bc1dcbe4152cce4a`. The human-led continuation completed the E–G investigations and reported the runs below. No production code changed during the walkthrough.

### Completed investigations

- **A — Compiler entry and decoding.** Public bytecode compilation routes through the symbolic compiler. The current decoding and compilation path supports six sequential opcodes. Decoder reachability and broader opcode support remain future architectural concerns. [H-WALK-001](human-led-walk-findings.md#h-walk-001--decoder-mixes-decoding-and-reachability)
- **B — Opcode generation.** Reviewed the symbolic PUSH1 capacity guard and stores for the 32-byte EVM word. This was inspection of generated RV64 instructions. The later execution review adds focused runtime evidence.
- **C — Symbolic composition.** Reviewed `compose()`, `emit()`, `assemble_image()`, `verified_program()`, and index-based symbol names. `asm-rs` resolves the assembly, and dependency source shows that version `0.2.0` rejects duplicate named labels. [H-WALK-002](#h-walk-002--symbol-naming-and-namespace-ownership)
- **D — Final-image verification.** Reviewed `verify_declared_transfers()`, `verify_transfer()`, `decode_transfer()`, `verify_block()`, and `verify_final_image()`. These checks decode machine-code transfers, compare their targets with declarations, and independently enforce fixed-layout destinations. Reviewed the fixed 34-word ADD and SUB core byte-identity checks and existing mutation tests. Commit `ffe67f3` added `matching_push1_continuation_cannot_skip_the_next_fragment`. It checks that declaration verification accepts a matching redirection while fixed-layout verification rejects the skipped fragment. H-WALK-003 records a prior focused pass. The A–D handoff confirmed the committed test by source inspection. [H-WALK-003](#h-walk-003--independent-continuation-verification)

### E–G evidence from the November 21 continuation

**E — Execution and ADR-001 ABI, checked in source.** The symbolic compiler obtains publisher PCs from assembler-resolved labels using `AssemblyResult::label_address()` and verifies the final image before returning the executable artifact. The orchestrator loads the image, initializes the guest and optionally seeds its stack. It runs the body until the PC reaches a resident publisher, then runs that publisher to `EXIT_PC`. The rvemu executor checks stop PCs before fetch, calls `cpu.execute()` for one instruction at a time and counts only completed instructions. Instruction-limit exhaustion and backend faults return executor errors.

The guest uses `s0` as the EVM stack base and `s1` as the next free slot, with 32 bytes per word. The publisher stores final `s1`, exception code and terminal status in that order. It reaches `EXIT_PC` via `jalr zero, -4(t1)`, independently of `ra`. After the publisher run returns, `read_guest_result()` validates the metadata, checks live `s0` against the ABI base and live `s1` against the published pointer, then reconstructs the stack. The isolated arithmetic runner places operand/result pointers in `a0`–`a2` and seeds `ra` with its halt PC. The guest uses a stack-aware wrapper and reserves `ra` for the leaf's local return. The focused runs below exercise selected outcomes. The extent of runtime fault coverage remains open. [ADR-001](../../docs/adr/001-evm-guest-state-abi.md) · [Composer](../../crates/ferreth-compiler/src/compiler/symbolic.rs) · [Orchestrator](../../crates/ferreth-compiler/src/orchestrator.rs) · [Executor contract](../../crates/ferreth-compiler/src/rv64/executor/mod.rs) · [rvemu executor](../../crates/ferreth-compiler/src/rv64/executor/rvemu.rs) · [Memory layout](../../crates/ferreth-compiler/src/rv64/memory.rs) · [Guest ABI](../../crates/ferreth-compiler/src/rv64/guest.rs) · [Publisher](../../crates/ferreth-compiler/src/compiler/publisher.rs) · [Isolated runner](../../crates/ferreth-compiler/src/execution.rs)

**F — ADD/SUB, checked in source and exercised by the reported integration runs.** Both use the shared wrapper, which checks for two words before any operand access. It passes the top item via `a0`, the next item via `a1`, and writes into the second item's slot through `a2`. SUB therefore computes top minus next. The cores process four little-endian 64-bit limbs, propagating carry or borrow and discarding the final bit for arithmetic modulo 2²⁵⁶. The wrapper's `jal ra, leaf` returns to the `s1` decrement by 32. Test assertions cover normal publication, wraparound and cross-limb propagation, explicit SUB order, empty and one-item underflow, preservation of live stack and `s1` on those underflows, and instruction counts of 42+7 on success and 3+8 on underflow in the singleton guests. The core fingerprint fixtures are ADD `0xf1fff32ff47962f3` and SUB `0x9f008f320eeb7a7b`. The fingerprint values come from the unit-test fixture. [ADD generator](../../crates/ferreth-compiler/src/rv64/opcodes/add.rs) · [Shared wrapper](../../crates/ferreth-compiler/src/rv64/opcodes/arithmetic.rs) · [Leaf cores](../../crates/ferreth-compiler/src/lib.rs) · [ADD tests](../../crates/ferreth-compiler/tests/rv64_guest_add_orchestrator.rs) · [SUB tests](../../crates/ferreth-compiler/tests/rv64_guest_sub_orchestrator.rs) · [Fingerprint fixtures](../../crates/ferreth-compiler/src/compiler/symbolic/tests/arithmetic.rs)

**G — Conformance, checked in source and exercised by the reported reference runs.** The transaction-based differential suite uses REVM 43.0.3 with Cancun and a transaction gas limit of 1,000,000. It combines directed cases with a deterministic, stack-aware generator for the supported subset. Its fixed seed `0xFE77_E7A0_0000_0001` drives quotas of 1,000 valid and 100 intentionally invalid programs. The comparison checks expected terminal success or halt, Ferreth's own status and exception code, and stack equality on success. Exceptional stacks are diagnostic. A dedicated empty-stack DUP1 regression pins the REVM `StackOverflow` label and the Ferreth `StackUnderflow` code. EELS reports `StackUnderflowError` for the same empty-stack instruction. The labels differ for an agreed exceptional halt. EELS runs an isolated frame, so its gas observations belong to that reference execution alone. [REVM differential tests](../../crates/ferreth-conformance/tests/rc_revm_conformance.rs) · [Conformance library](../../crates/ferreth-conformance/src/lib.rs) · [Conformance research](../../docs/research/ferreth-evm-conformance.md) · [EELS CLI](../eels-cancun/run_frame.py) · [EELS setup and pin](../eels-cancun/README.md) · [Direct REVM frame experiment](../revm-frame/README.md#2-empty-stack-dup1)

| November 21 command | Reported observation |
|---|---|
| `cargo test -p ferreth-compiler --test rv64_guest_add_orchestrator` | Passed |
| `cargo test -p ferreth-compiler --test rv64_guest_sub_orchestrator` | Passed |
| `cargo test -p ferreth-conformance --test rc_revm_conformance` | Passed, including the dedicated empty-stack DUP1 regression |
| `uv run run_frame.py --code 600560030100 --gas 100` | Success `True`, stack `[8]`, frame gas consumed `9`, remaining `91`, PC `6` |
| `uv run run_frame.py --code 80 --gas 100` | Success `False`, `StackUnderflowError`, remaining frame gas `0`, stack `[]` |
| `cargo run -q -p ferreth-conformance -- bench guest --format json` | Opcode and program instruction metrics matched the full tracked guest baseline after removing warning lines and sorting JSON keys. See [H-WALK-005](human-led-walk-findings.md#h-walk-005--guest-benchmark-warnings-on-json-stdout). |

Cargo commands were run from the repository root. EELS commands were run from `research/eels-cancun/` with an empty initial stack. The command outcomes come from the human session record. The documentation pass checked source, pins and references. Historical Phase 2 workspace totals, opt-in probes and the arithmetic benchmark remain separate evidence.

### Resume at the human exit gate

Bring the source traces and this evidence ledger to [the five challenges](#10-the-human-led-walks-five-deliberate-challenges) and [the exit gate](#12-the-exit-gate-what-we-decide-after-the-walk). Review the [findings notebook](human-led-walk-findings.md) for the Phase 3 candidates and their pending decisions.

---

## 0. Five-minute re-entry: where are we?

### What Ferreth is

Ferreth is an **experimental Rust EVM-bytecode → RV64 compiler and execution/conformance system**, not a full Ethereum Virtual Machine and not a production-ready engine. Its current supported instruction subset is:

| EVM instruction | Purpose in this project | Important edge |
|---|---|---|
| `STOP` | Normal termination | EOF also ends normally; unreachable trailing bytes must not be decoded as reachable instructions |
| `PUSH1` | Write a 256-bit stack word containing a byte immediate | Truncated immediate at EOF is right-zero-padded; full stack overflows |
| `POP` | Remove the top stack item | Empty stack underflows without reading memory |
| `DUP1` | Duplicate the top 256-bit word | Empty underflows; full stack overflows |
| `ADD` | 256-bit modular sum | Requires two operands; carries across four 64-bit limbs |
| `SUB` | 256-bit modular subtraction | Requires two operands; borrow/order and wraparound matter |

This project does **not yet** claim full EVM control flow (`JUMP`, `JUMPI`, `JUMPDEST`), complete EVM memory and storage semantics, gas/OOG accounting, or `KECCAK256`. Work related to ADR-002 gas metering belongs **after** this walk and Phase 3 stabilization, subject to later planning.

### Why Phase 2 happened

Historically, Ferreth had several overlapping ways to link small RV64 guests: a PUSH1-only linker, a mixed-opcode linker, and isolated arithmetic linkers. Some constructed assembly text and then patched displacements or machine-code words manually. **LF-026** exposed a real problem: a 47-`PUSH1` program required a distant JAL displacement (`+2056` or `+2060` bytes), and the old numeric-JAL assembler path rejected it. **LF-027** concerned incorrect signed backward-JAL encoding in the former manual strategy.

The Phase 1 experiments explored symbolic assembler targets; Phase 2 built a single composer, added independently decoded final-image verification, moved six opcodes to the symbolic path privately, then switched the public APIs in Phase 2.4. Phase 2.5 checked that the old linkers had already become `#[cfg(test)]`-only oracles and made **documentation/attribute cleanup**, rather than deleting valuable independent tests.

### What Phase 2 reported

- Public `compile_evm_bytecode` and `compile_push1_program` used `symbolic::compile` and `symbolic::compile_push1_sequence` respectively at the Phase 2 cutover. The branch subsequently retired the specialized public PUSH1 compile and sequence execution APIs (`0f70552`, `4a61790`). The current public whole-program route is bytecode-based. [Compiler entry](../../crates/ferreth-compiler/src/compiler.rs) · [Orchestrator entry](../../crates/ferreth-compiler/src/orchestrator.rs)
- Active guest production uses one symbolic composer, one assembled final image and final-image verification before execution.
- Legacy whole-image linkers are isolated under `#[cfg(test)]` and retained as independent oracles.
- At the Phase 2.4/2.5 gate: **215 discovered, 212 passed, 0 failed, 3 ignored**; all three opt-in capacity probes passed separately.
- The four 47-PUSH LF-026 public regression cases passed; benchmarks matched the accepted JSON baselines.
- The public *resident exceptional publisher* count is `0`, `8`, or `16` words. **Dynamic execution** uses only one terminal publisher: normal `7` instructions or exceptional `8` instructions.
- The 34-word ADD and SUB leaf-core fingerprints are `0xf1fff32ff47962f3` and `0x9f008f320eeb7a7b` (FNV-1a64 as reported).

Phase 2 test totals, capacity results and benchmark comparisons describe their recorded revisions. The November 21 ledger identifies the later source checks and focused runs. [Public cutover review](phase-2.4-public-cutover.md) · [Final cleanup](phase-2.5-final-cleanup.md)

### What Phase 2.5 actually changed

According to the handoff, it removed a stale `#[allow(dead_code)]` annotation from the active symbolic module, narrowed `dead_code` allowances in `rv64/verify.rs` to test-only variants, corrected outdated PUSH1 Rustdoc, and created/updated research documentation. It **did not rewrite production algorithms**. Test-only legacy linkers and duplicated test-only JAL helpers remain intentionally. This is not a failed cleanup; deleting the only independent oracles would have made the system less trustworthy.

---

## 1. Start with Git, not an agent

At the repository root:

```bash
git status --short --branch
git log -5 --oneline --decorate
git branch --merged
git branch --no-merged
git diff --check
```

**Checkpoint 1 — record without editing:**

- [ ] Current branch and HEAD recorded: `________________________`
- [ ] Phase 2.4 cutover commit found (reported `3eb4a94`): `________________`
- [ ] Phase 2.5 commit found (if already created): `________________`
- [ ] Working tree condition: `________________`
- [ ] Decide whether this is a clean review session; do not perform broad merges or cleanup here.

If you are not on the expected branch or worktree is dirty, **stop before writing changes** and inspect why. Do not use `git reset --hard`, `git clean`, or blanket staging to get a clean state.

**Recommended reading: source of truth, in order**

1. `AGENTS.md` — repository rules.
2. `docs/bitacora/README.md` and the relevant November 2026 entry — how to navigate the historical context.
3. `research/guest-linking/phase-2.4-public-cutover.md` — latest public acceptance evidence and RF-026/RF-027 repair.
4. `research/guest-linking/phase-2.5-final-cleanup.md` — retained legacy oracles, test matrix and known debt.
5. `research/guest-linking/phase-2.3d-integration-review.md` — private six-opcode proof obligations and prior defects RF-023–RF-025.
6. `research/guest-linking/ferreth-linking-architecture.md` — architecture/roadmap; distinguish historical design from current implementation.
7. [ADR-001](../../docs/adr/001-evm-guest-state-abi.md) governs the guest ABI. [ADR-002](../../docs/adr/002-cancun-evm-gas-metering.md) remains proposed.

If any document contradicts current code, mark the contradiction; don't silently fix it during the reading session.

---

## 2. The map you must be able to explain

The expected production pipeline is:

```text
EVM bytecode
                  │
                  ▼
Public compiler entry (compiler.rs)
                  │
                  ▼
Decoder / symbolic frontend (compiler/symbolic.rs)
  • reachable STOP / EOF / truncated PUSH1 / diagnostics
                  │
                  ▼
Opcode-specific generators (rv64/opcodes/*)
  • fixed guest body + labels + declared transfer sites
                  │
                  ▼
Typed SymbolicFragment contract
  • source, entry, external roles, local transfers, layout
                  │
                  ▼
Single symbolic compose step
  • combines fragments, appends required publishers
  • assembles once via asm-rs
                  │
                  ▼
Final RV64 image + bound label PCs
                  │
                  ▼
Independent final-image verifier (rv64/verify.rs)
  • decode actual instructions and check control-flow/layout
                  │
                  ▼
CompiledRv64Program / executable artifact
                  │
                  ▼
Orchestrator → rvemu executor → ADR-001 publication
                  │
                  ▼
Observed stack, status, error, registers, counts
                  │
                  ▼
Independent conformance / runtime expectations
```

### Mandatory distinction

**Compiler/generator** decides what bytes to emit. **Assembler** resolves symbols and emits encoded RV64 instructions. **Verifier** inspects the final image and rejects mismatches; it must **not** repair bytes or generate linker addresses. **Executor** runs the guest and reports state. **Tests/oracles** must check facts that are not merely copied from the same candidate.

If you can only recite this diagram but cannot locate the responsibility in source, the walk is not complete.

---

## 3. Session A — Enter through the public APIs

**Read:** `crates/ferreth-compiler/src/compiler.rs`, then `crates/ferreth-compiler/src/compiler/symbolic.rs`.

**Goal:** trace two inputs without invoking the emulator:

```text
Bytecode route: 60 05 60 03 01 00
PUSH1-only bytecode: 60 05 60 03
```

The first means `PUSH1 5; PUSH1 3; ADD; STOP` and should finish with `[8]`. The second reaches EOF with `[5, 3]` in bottom-to-top order. Both enter through the bytecode API.

**Write down, from code:**

1. Which public function receives each input? Which internal function is called next?
2. Where is reachability terminated at the first `STOP`? What if bytes follow it?
3. Where is incomplete `PUSH1` right-padded? Does `60` at EOF become `PUSH1 0`?
4. How is unsupported `PUSH2` reported — opcode and original byte offset? What about an unknown opcode after `STOP`?
5. Which function owns the decision to append Normal at EOF?
6. Does any production branch still invoke `compile_*_legacy`?

**Commands for finding calls, not proving them:**

```bash
rg -n 'pub fn compile|fn compile|compile_evm_bytecode|decode_detailed' crates/ferreth-compiler/src
rg -n 'compile_.*legacy|link_guest_arithmetic|encode_jal_zero' crates/ferreth-compiler/src
```

**Checkpoint A:** draw the entry-point call chain in your own words, including how the decoder consumes PUSH1 immediates without reinterpreting them as opcodes.

---

## 4. Session B — Follow one opcode into machine code

**Read:** `crates/ferreth-compiler/src/rv64/opcodes/` (actual file names in tree), especially PUSH, POP, DUP and arithmetic; inspect the type implementing the fragment contract in `compiler/symbolic.rs`.

Choose `PUSH1 0x05` and track it **without taking the generator's comments on faith**:

- [ ] Locate stack-capacity preflight: which registers/pointers are used? Is the comparison unsigned?
- [ ] Locate the four 64-bit limb stores representing one 256-bit EVM word.
- [ ] Locate the `s1` update by 32 bytes on success.
- [ ] Locate the path to Overflow **before** any stack write on failure.
- [ ] Locate each local label and exceptional external role declaration.
- [ ] Locate the fragment's expected layout or transfer descriptor.

Repeat the critical steps for `POP` and `DUP1`:

- `POP` empty stack: no memory read; Underflow status.
- `DUP1` empty: Underflow; full: Overflow; normal: preserve original top and append an exact 256-bit copy.

**Questions that should feel uncomfortable:**

- Does the declaration of a transfer site originate from the same generator that emits it? If so, what independent evidence detects a generator that declares its own wrong jump?
- What is checked independently after assembly versus merely asserted by metadata?
- Could a fragment declare a transfer instruction belonging to a *different* fragment? This was RF-025.
- Which invariants rely on the ABI's initial register state and are not proven for arbitrary RV64 states?

**Checkpoint B:** hand-sketch the success and fault control flow for one `PUSH1` fragment and a `DUP1` fragment. Identify the exact guard that prevents a fault path from modifying stack data.

---

## 5. Session C — Understand the composer and the linker contract

**Read:** `compiler/symbolic.rs` and `compiler/publisher.rs` (or their actual paths).

Do not begin with the 47-PUSH program. First understand **two adjacent fragments**:

```text
PUSH1 0x05  →  POP  →  normal EOF
```

Ask:

1. How does fragment N name its continuation into fragment N+1?
2. How are symbol names kept unique across repeated opcode occurrences?
3. Who gathers required exceptional roles? Does unused code get emitted?
4. In which physical order are publishers appended? Expected: Normal, Underflow if required, Overflow if required.
5. How does the composer obtain actual publisher PCs after assembly — resolved labels or guessed offsets?
6. Does `compose` contain opcode-specific linking branches, patch a numeric JAL, or edit emitted bytes? It should not.
7. What causes a duplicate label, missing target, out-of-fragment declaration, or oversized image to be rejected?

**Publisher table you should derive by reading source:**

| Sample program | Expected resident roles | Resident exceptional words | Executed terminal publisher |
|---|---|---:|---|
| Empty input / STOP alone | Normal | 0 | Normal, 7 |
| PUSH1 only | Normal, Overflow | 8 | Normal 7 or Overflow 8 |
| POP only | Normal, Underflow | 8 | Normal 7 or Underflow 8 |
| PUSH1 then POP | Normal, Underflow, Overflow | 16 | Normal 7 or selected exceptional 8 |

These are **contract expectations**. Confirm them against actual program metadata, compiled bytes and tests. Do not assume both exceptional bodies execute because both are resident.

### The long-jump exercise — LF-026

Examine the exact four original public 47-PUSH tests in `crates/ferreth-compiler/tests/rv64_push1_program.rs` and the private independent transfer decoder.

- With 47 pushes and implicit EOF, the first distant JAL at `ENTRY_PC + 40` has expected displacement `+2056`.
- With 47 pushes followed by STOP, expected displacement is `+2060`.
- Successful execution: 47 stack words each equal to 2; `s1=0x801015e0`; dynamic body/publisher counts `470+7` or `471+7`.
- Seeded full-stack overflow: preserve 1,024 words; `s1=0x80109000`; exception code 1; counts `4+8`.

**Crucial:** do not accept an assembly label or metadata PC as proof of the target. Independently decode the signed J-type immediate from the *final word*, calculate `source PC + displacement` and compare it to the intended role address.

**Checkpoint C:** explain in one paragraph why symbolic JAL fixed LF-026 and why a signed-immediate mistake (LF-027) could evade simplistic positive-only tests.

---

## 6. Session D — Verify the verifier

**Read:** `crates/ferreth-compiler/src/rv64/verify.rs` plus `rv64/verify/tests.rs` and `tests/rv64_final_image.rs`.

This is the highest-priority reasoning session.

**Separate three questions:**

1. *Structural integrity:* Is the final machine code a well-formed aligned image inside the ADR-001 code reservation?
2. *Declared control-flow integrity:* Do decoded JAL/B-type/JALR instructions have valid families, destination registers, sites, signed targets and expected roles?
3. *Semantic integrity:* Does each supported opcode actually perform the intended 256-bit stack operation on all relevant states?

The current verifier is **not a general proof of arbitrary RV64 semantic correctness**. The reports say it checks fixed layout, exact protected arithmetic words and publishers, transfer sites and ownership, while other straight-line instruction semantics rely on runtime and independent fixture coverage. Verify these boundaries rather than calling everything “verified.”

### Reconstruct the negative controls

Locate tests that intentionally change source/declarations or machine words to check rejection. Include at least:

- An incorrect signed backward JAL.
- A JAL to an unaligned/invalid target.
- A swapped publisher role with otherwise plausible PCs.
- A missing publisher body or invalid zero-address declaration (RF-024).
- A declaration whose instruction site belongs to another fragment (RF-025).
- The PUSH1 continuation that skips a live fragment even when its declaration agrees with the wrong target. See [H-WALK-003](#h-walk-003--independent-continuation-verification).
- An incorrect arithmetic call target or return anchor.

Do not assume these are the exact test function names. Locate the actual tests and record names/results.

**Adversarial thought experiment:** “If the generator and transfer declaration both make the **same** mistake, which independent test or verifier invariant catches it?” Write one concrete scenario and its expected failure. If none exists, record a gap; do not invent a guarantee.

**Checkpoint D:** produce a trust-boundary diagram with two columns: “checked by decoding final bytes” and “still dependent on runtime/oracle evidence.” This is a mandatory output of the walk.

---

## 7. Session E — Trace execution and ADR-001 publication

**Read:** `crates/ferreth-compiler/src/orchestrator.rs`, executor under `src/rv64/executor/`, guest memory/register layout (e.g., `rv64/guest.rs`, `rv64/memory.rs`), and ADR-001.

Trace **one successful** and **one failing** guest:

```text
Success:   60 05 60 03 01 00     # PUSH1 5, PUSH1 3, ADD, STOP
Underflow: 50                    # POP from empty stack
```

Record:

- Where `s0` and `s1` are initialized and which registers the stack access and exit paths actually depend on. Compare the isolated runner’s explicit `ra` setup.
- Where stack lower/upper bounds and reserved code/exit regions are enforced.
- What the executor considers a publisher PC, and why it must accept the actual role-specific PC set.
- How the guest reaches the exit sentinel, why the sentinel is not counted as a guest body instruction, and what `ra` means for guest-local arithmetic leaf returns versus host exit.
- Exact order of publication stores: final pointer, exception code, terminal status last.
- What a normal result and an exception report to the public orchestrator.
- Whether a failure could partially change the EVM stack before underflow/overflow is detected.

**Checkpoint E:** hand-draw both instruction paths through the selected publisher and sentinel. Check the expected stack and terminal status against an execution result, not just a compiled artifact.

---

## 8. Session F — Understand ADD/SUB's local calls and 256-bit behavior

**Read:** arithmetic generator/wrapper and protected leaf routines; compare `compiler/symbolic/tests/arithmetic.rs` with standalone arithmetic oracle checks.

The Phase 2.3d report describes a **43-word complete arithmetic fragment**, including a **34-word protected leaf core**. Typical normal path executes **42 body instructions + 7 Normal publisher instructions**; the underflow path takes **3 + 8**. The local `jal ra, leaf` must return to the correct stack-pointer-update instruction without clobbering the host sentinel behavior.

Reconstruct for `ADD` and `SUB`:

1. Which stack element is `a` and which `b`? Read the implementation; don't infer from Rust names.
2. What checks ensure a depth of at least two before any operand access?
3. How are the four 64-bit limbs loaded/written to represent 256 bits?
4. Where does carry propagate in ADD and borrow propagate in SUB?
5. Where is the leaf call encoded, and which instruction is the return target?
6. How is `s1` decremented after successful arithmetic?
7. Does the isolated public arithmetic entry produce the same image as the retained **independent legacy isolated linker**, or is the test comparing the symbolic path to itself?
8. What exactly do the FNV hashes cover (protected leaf words, not necessarily whole guest image)?

**Suggested concrete arithmetic oracles:** `MAX + 1 = 0 mod 2^256`; subtraction underflow/wraparound; carry/borrow across limbs; repeated ADD/SUB; one-operand underflow without stack writes.

**Checkpoint F:** describe the guest-local call/return and explain why `ra` is safe despite its special role in host exit. Provide the actual core hash values as verified from test output or source fixtures.

---

## 9. Session G — Conformance, benchmarks and accepted limits

**Read:** `crates/ferreth-compiler/tests/`, conformance crate tests, `docs/runbooks/benchmark.md`, `runs/phase-0/guest-benchmarks.json` and `runs/rv64-arithmetic.json`.

### Runtime and conformance

Clarify which tests compare public behavior against an **independent reference** (e.g., REVM), and which merely compare symbolic to legacy or run the generated image through the same internal executor. Use the November 21 ledger for the two EELS CLI runs and the linked frame research for earlier captures.

Use the empty-stack DUP1 regression to explain the diagnostic distinction. Keep the observed labels and the shared exceptional termination explicit.

### Capacity is a stated implementation boundary, not an Ethereum consensus rule

- ADR-001 executable interval: reported `[0x80000000, 0x800ffffc)`, maximum **1,048,572 bytes**.
- Under the pinned `asm-rs` defaults, POP: **49,999** occurrences accepted at the reported 100,000-label limit; 50,000 rejected by assembler resources.
- DUP1: **16,382** accepted; 16,383 exceeds image reservation.
- PUSH1: **23,829** accepted, 23,830 exceeds image reservation (with EOF or STOP).

Don't reopen label-density optimization during this walk. Instead ask whether rejection is deterministic, has the correct `Link`/`Layout` class, and produces no partial artifact.

### Full validation commands for the exit gate

The ledger above covers the focused November 21 runs. The commands below reproduce the wider Phase 2 gate on the current branch. Run from the repository root with a fresh target directory.

```bash
export CARGO_TARGET_DIR=target/human-led-walk-validation
cargo fmt --all -- --check
cargo clippy --frozen --workspace --all-targets -- -D warnings
cargo test --frozen --workspace -- --list
cargo test --frozen --workspace --no-fail-fast
cargo test --frozen -p ferreth-compiler --lib capacity -- --ignored --nocapture --test-threads=1
cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1
cargo run --frozen -q -p ferreth-conformance -- bench guest --format json
cargo run --frozen -q -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json
```

Reconcile current test discovery, executed tests and ignored tests. Phase 2 historically recorded **215 = 212 + 3**, with separate passes for all three opt-in probes. For benchmark comparisons, preserve raw stdout and account for the warning prefix before comparing complete parsed JSON objects to the tracked baselines.

**Checkpoint G:** create a table of claims, independent source, invocation and observed result. Flag missing independent evidence even if the test suite is green.

---

## 10. The human-led walk's five deliberate challenges

Do not outsource answers to these until you have attempted them from code.

### Challenge 1 — Responsibility boundaries

Can we identify **one owner per responsibility** for decode, lowering, fragment generation, composition, publisher creation, verification, execution and conformance? Are any two layers silently making incompatible assumptions?

### Challenge 2 — A shared mistake

Find at least one realistic error that generator **and** metadata declaration could agree on. Locate the independent check that prevents it. If there isn't one, record a focused test proposal for Phase 3.

### Challenge 3 — Failure atomicity

For PUSH1, POP, DUP1, ADD and SUB, prove from the instruction sequence that exceptional paths do not corrupt live stack contents or publish an inconsistent final pointer/status. Distinguish “proved by inspection” from “observed in tests.”

### Challenge 4 — The seventh-opcode thought experiment

Without implementing anything, trace what would be required to add a simple seventh opcode. Which modules and tests change? Does composer require a new opcode-specific branch? If yes, challenge the architecture. Do **not** turn this thought experiment into new opcode work.

### Challenge 5 — The first version tag

What exactly could the project truthfully promise at `v0.1.0`? Write an explicit supported-domain statement and exclusions. “Verified symbolic linker” must not be misconstrued as formally verified, complete EVM or production safety.

---

## 11. Write findings as you go (not at the end)

Record new findings in the [review notebook](human-led-walk-findings.md). H-WALK-002 and H-WALK-003 remain below with their original review context. Suggested entry format:

```markdown
## H-WALK-001 — Short, neutral title

- **Session / date / reviewer:**
- **Claim under examination:**
- **Source / function / lines / commit:**
- **Input specimen and initial state:**
- **My expected behavior before running:**
- **Observed machine words / PCs / stack / status:**
- **Independent evidence (or absence):**
- **Classification:** Blocker / phase-3 hardening / accepted limitation / future work
- **Action proposal:**
- **Status:** Open / confirmed / rejected / deferred
```

Prefer *one concrete counterexample* over paragraphs of general concern. If a question cannot be settled by inspection, write the smallest test that would settle it **as a proposal**, and save implementation for Phase 3 unless the worktree has a demonstrable critical defect that requires intervention.

## H-WALK-002 — Symbol Naming and Namespace Ownership

- **Evidence:** `crates/ferreth-compiler/src/compiler/symbolic.rs` enumerates decoded instructions and passes occurrence indices to opcode generators. Generators use deterministic, opcode-prefixed named labels, e.g. `guest_push1_{index}` / `push1_overflow_{index}`, `guest_pop_{index}` / `pop_underflow_{index}`, `guest_dup1_{index}` and its underflow/overflow labels, and `guest_arithmetic_{index}` with arithmetic-local labels. ADD and SUB share `generate_symbolic_arithmetic`.
- **Ownership:** `compose()` rejects duplicate fragment entry symbols; `emit()` concatenates fragment sources and required publishers; `assemble_image()` assembles the complete source through `asm-rs`. The composer does not centrally allocate local labels, and frontend continuation generation shares opcode-specific entry-label conventions with generators.
- **Assembler evidence:** `Cargo.lock` pins `asm-rs 0.2.0`. In that dependency's `src/linker.rs`, `Linker::add_label()` rejects duplicate named labels with `AsmError::DuplicateLabel`; the dependency has a `duplicate_label_error` negative unit test. Numeric labels may be redefined. This is dependency source evidence, not a Ferreth negative test for duplicate local labels.
- **Distinct safeguard:** `crates/ferreth-compiler/src/rv64/verify.rs::verify_declared_transfers()` sorts transfer declarations by site PC and rejects duplicate declared sites. This does not reject duplicate label definitions or prove that a transfer targets the intended continuation.
- **Finding / classification:** Occurrence indices and generator-specific prefixes prevent ordinary collisions in the current six-opcode subset, while duplicate named labels are ultimately rejected by `asm-rs`. This is reasonable for v0.1, with no demonstrated incorrect binary or active collision, but namespace uniqueness is not guaranteed by a centralized Ferreth allocator and the shared naming conventions are a maintainability and extensibility concern. **Phase 3 hardening candidate — non-blocking for v0.1.**
- **Continuation question:** A unique, valid symbol may still be the wrong existing continuation. [H-WALK-003](#h-walk-003--independent-continuation-verification) resolves the three-fragment PUSH1 case. The broader verification design for future dynamic control flow remains open.
- **Decision:** No refactor or centralized allocator during this walkthrough; preserve naming conventions, continue reviewing final-image transfer validation and independent negative controls, and reassess namespace ownership in Phase 3 if evidence warrants it.

## H-WALK-003 — Independent Continuation Verification

- **Evidence:** `verify_declared_transfers()` scans the assembled fragment region in four-byte increments, decodes JAL/BRANCH/JALR sites, and compares encoded kind, registers, and resolved destination against declarations. It rejects undeclared, duplicate, and unmatched declarations. `verify_final_image()` separately verifies each expected fixed layout, including required transfer sites and destinations, publisher ordering/boundaries, and executable-image end.
- **Adversarial control:** `matching_push1_continuation_cannot_skip_the_next_fragment` assembles three sequential PUSH1 fragments after redirecting the first continuation from `push1_continue_0` to `guest_push1_2` and updating its declaration to match. The test confirms assembly succeeds and `verify_declared_transfers()` accepts the consistent transfer, while `verify_final_image()` rejects it: `transfer at 0x80000024 targets 0x80000058, expected 0x8000002c`. The focused test passed.
- **Finding / classification:** Declaration consistency and structural correctness are complementary properties. A consistent but incorrectly directed PUSH1 continuation cannot bypass fixed-layout verification. **Verified correctness property — no v0.1 blocker identified.** This closes the H-WALK-002 continuation-independence question for this three-fragment PUSH1 case; it does not establish arbitrary EVM control-flow or ordinary RV64 semantic correctness. Layout identities still derive from requested operations, and dynamic EVM JUMP/JUMPI require separate verification design.
- **Decision:** Preserve both verification layers and the adversarial regression; no production refactor is required. Continue the Human-Led Walk with remaining verifier boundaries and negative controls, and revisit generalized control-flow verification when implementing JUMP/JUMPI.

### Findings categories

| Category | Meaning | Immediate effect |
|---|---|---|
| **Blocker** | Incorrect supported-domain semantics, unverifiable control flow, broken ABI, loss of independent evidence, or an uncontrolled failure | No stable baseline/tag until resolved |
| **Phase 3 hardening** | Missing focused test, inadequate diagnostics, duplicated assumptions, cleanup with bounded impact | Prioritize in stabilization plan |
| **Accepted limitation** | Explicit resource bound or out-of-scope EVM behavior, correctly rejected/documented | Document; do not expand scope |
| **Future work** | Gas, new opcodes, dynamic jumps, KECCAK256, performance experiments | Keep out of Phase 3 unless separately approved |

---

## 12. The exit gate: what we decide after the walk

The walkthrough is ready to bring to the human exit-gate review. Passing the gate still requires the maintainer to record answers to all five challenges and decide how to close each evidence gap below.

- **Responsibility boundaries:** Confirm the owner map against the bytecode pipeline and the namespace concern in H-WALK-002.
- **Shared mistake:** Use H-WALK-003 as the concrete adversarial control. Record the independent JAL calculation and the Session D trust-boundary diagram, including the limits of fixed-layout checks. Record which remaining suggested negative controls have supporting evidence or still need a focused check.
- **Failure atomicity:** Reconcile guards, stack writes, final `s1` and publication for PUSH1, POP, DUP1, ADD and SUB. Identify where the review relies on instruction inspection, current focused runs or earlier tests. Close the two Checkpoint E specimen traces with Ferreth runtime results or cite matching evidence.
- **Seventh opcode:** Record the modules and tests the hypothetical opcode would touch and whether the composer would need an opcode-specific branch.
- **First version tag:** Write the supported-domain claim and exclusions, including resource limits and EVM features outside the current subset.

Current-revision workspace format/lint, full discovery and test totals, opt-in capacity reruns and the arithmetic benchmark comparison are pending on the supplied November 21 evidence. The focused ADD/SUB and REVM runs, two EELS observations and guest benchmark payload comparison are recorded above. Decide the disposition of each pending check explicitly and record ownership for H-WALK-004 and H-WALK-005 before closing the walk.

### Maintainer sign-off

- [ ] We can trace EVM input to final emitted RV64 words, and then to observed execution state.
- [ ] We can independently decode at least one JAL and verify its resolved target.
- [ ] We can explain how all publisher roles are selected and why status is published last.
- [ ] We can explain the arithmetic local call/return and exact 256-bit stack effects.
- [ ] We know exactly where final-image verification is independent and where it is not.
- [ ] We checked at least one positive and one adversarial control in source/tests.
- [ ] We identified all remaining production-versus-test-only legacy distinctions.
- [ ] We reconciled test discovery, opt-in probes and baseline benchmarks.
- [ ] Every important finding has a classification and follow-up owner/decision.
- [ ] We can state the precise, limited claim that a future `v0.1.0` would represent.

**Possible conclusions:**

1. **Ready to plan Phase 3:** bounded list of concrete stabilization tasks, no hidden architectural questions blocking the supported domain.
2. **Walk incomplete:** one or more critical code paths still not understood; continue reading rather than requesting a broad agent rewrite.
3. **Architecture blocker found:** write the shortest reproduction and pause the stability/tag decision until investigated.

**Do not tag `v0.1.0` at the end of this walk.** The milestone candidate is **after** the human findings have informed Phase 3 and all release-blocking findings are closed.

---

## 13. First hour: a literal starting itinerary

For a cold restart, do just this:

**0–10 minutes — Establish location in history**  
Open this file. Run `git status`, inspect HEAD, read the Phase 2.4 and 2.5 summaries and locate ADR-001. Write down three facts you believe and one you want to challenge.

**10–25 minutes — Open the public API**  
Read only the relevant portions of `compiler.rs` and `compiler/symbolic.rs`. Follow `60 05 60 03 01 00` by hand. Draw call arrows. Do **not** open the entire repo at once.

**25–40 minutes — Read PUSH1 body**  
Identify the stack check, stores, pointer update and exceptional jump. Find exact fragment declarations. Sketch the instruction flow.

**40–55 minutes — Read composer/publishers**  
Trace the two-fragment example to symbol resolution and publisher PC binding. Distinguish `Normal`, `Underflow`, `Overflow` and resident-vs-dynamic counts.

**55–60 minutes — Write your first finding**  
Record a question about ownership, a claim verified from code, and the exact next source file to inspect. Stop here if attention is fading; progress in this walk is *understanding*, not volume of lines read.

### Where to resume on the following day

Use the [2026-11-21 handoff](#session-handoff--2026-11-21) to resume at the human exit gate.

---

## 14. Context anchors / historical references

These documents are navigation and historical evidence, **not substitutes for current source**:

- [Guest-linking research index](README.md) and [Phase 1 experiments](phase-1/)
- [Phase 2.3d integration review](phase-2.3d-integration-review.md), [Phase 2.4 cutover plan](phase-2.4-cutover-plan.md), [Phase 2.4 review](phase-2.4-public-cutover.md) and [Phase 2.5 cleanup](phase-2.5-final-cleanup.md)
- [Linking roadmap](ferreth-linking-architecture.md) and [conformance research](../../docs/research/ferreth-evm-conformance.md)
- [Benchmark runbook](../../docs/runbooks/benchmark.md), [guest baseline](../../runs/phase-0/guest-benchmarks.json) and [arithmetic baseline](../../runs/rv64-arithmetic.json)
- [ADR-001](../../docs/adr/001-evm-guest-state-abi.md) and [proposed ADR-002](../../docs/adr/002-cancun-evm-gas-metering.md)

---

**The promise of this walk is modest but serious:** by the end, the human maintainer should be able to say *what Ferreth compiles, how control flow is linked, how the guest exits, what is actually verified, and which risks remain*. Only then should Phase 3 attempt to stabilize it.

*One linker in production. Six supported opcodes. Independent evidence retained. Human eyes open.*

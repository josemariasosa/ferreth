# Ferreth — RV64 guest linking architecture

**Research record:** 2026-11-19  
**Baseline:** `main`, `b2e8b14c10484f0a19c828ef5ef1f8ca7529fa0b`  
**Status:** Research completed for this session; implementation and ADR-002 approval remain pending.  
**Resume:** Saturday, 2026-11-21. Start with the phases in section 8.

## 1. Purpose and engineering position

Ferreth is an experimental EVM bytecode-to-RV64 compiler. Its immediate objective is to establish a semantically correct, independently verifiable baseline for the supported Cancun instruction subset, against which later optimizations can be evaluated. Passing a finite corpus is evidence, not proof of complete EVM conformance.

**Correctness before optimization. Simplicity before generality.** Inefficient emitted instructions, duplicated arithmetic cores, unnecessary jumps and unused publishers are acceptable for now. Uncontrolled complexity in the compiler is not: it raises the cost of establishing correctness as coverage expands. The project aims to contribute useful experimental evidence to the broader EVM-to-RISC-V research effort, without claiming a definitive architecture or predicting Ethereum's eventual execution-layer design.

This investigation began with a manual, interactive source walk and a hand-calculated linking experiment. A separate Codex review independently traced the checkout, reproduced the experiment, inspected pinned dependencies and ran additional probes. The maintainer then reviewed the alternatives and chose a pragmatic direction: delegate textual and symbolic resolution to the existing assembler wherever verified to work. Exact historical assembly formatting is not a goal. This is a research direction, not approval of a particular implementation, API, or ABI change.

### Executive conclusion

Prepare guest composition before implementing gas. There are three guest linking paths, two owners of publisher source, and heterogeneous fragment contracts. This is no longer merely preventive work: 47 consecutive PUSH1 operations are rejected due to a numeric JAL parser limitation despite valid ISA displacements and successful Cancun reference executions. A duplicated hand-written signed JAL encoder also has a latent defect. Investigate symbolic composition with the pinned assembler, independently verify its output, and converge applicable guest entry points on one composer. Do not build a general linker or IR unless evidence demands it.

## 2. Evidence and scope

| Item | Recorded value |
|---|---|
| Branch | `main`, tracking `origin/main` |
| Commit | `b2e8b14c10484f0a19c828ef5ef1f8ca7529fa0b` |
| Initial and final tracked state | No staged or unstaged tracked changes |
| Pre-existing untracked file | `docs/research/ferreth-linking-architecture.md`, Git blob hash `5019bce49920ffefc4901d257464093597d5565d` before and after |
| Build | Rust and Cargo `1.98.1`, `aarch64-apple-darwin`, frozen workspace lockfile |
| Compiler dependencies relevant here | `asm-rs 0.2.0`, `rvemu 0.0.11`, `alloy-primitives 1.7.3` |
| Cancun references | REVM `43.0.3`, interpreter `43.0.2`, handler `43.0.3`. EELS pinned at `d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3` |

I read `AGENTS.md` first, then the engineering log, accepted ADR-001, proposed ADR-002, existing conformance research, compiler and executor sources, tests, benchmark code, dependency sources, and relevant Git history. I used a temporary `git archive` of the recorded commit for builds and execution. This also contained rvemu’s working-directory writes. I excluded the untracked research draft from the evidence base to preserve independence. [Engineering log](../../docs/bitacora/2026-11-18.md) (line 151) · [ADR-001](../../docs/adr/001-evm-guest-state-abi.md) (line 78) · [ADR-002](../../docs/adr/002-cancun-evm-gas-metering.md) (line 1)

**Evidence terminology.** *Observed* means I ran the command, test, or probe. *Source* means I traced the behavior in this revision or its pinned dependency. *Inference* identifies an architectural consequence. Scores and proposed changes are *judgments*. I exercised one host and backend, selected frame-level reference vectors, and the complete current Rust workspace suite. The proposed gas implementation and migration alternatives have no execution results.

The manual investigation inspected `main.rs`, `orchestrator.rs`, `compiler.rs` and `rv64/codegen.rs` and ran the PUSH1 translation and execution examples. The independent review supplied the additional repository, dependency, and runtime evidence below. The consolidated document does not represent a third execution of those experiments. All source line references are to the baseline revision. No project implementation was changed during this research.

## 3. Current architecture (as-is)

| CLI entry | Actual path | Product or observation |
|---|---|---|
| `translate --op ADD` | `Opcode::Add.assembly()` | Printable pointer-ABI arithmetic leaf, including display directives and `ret`. The command neither assembles nor executes it. SUB has the same translation interface. |
| `translate --op PUSH1 --value 2` | `generate_guest_push1(2)` | Assembled, **unlinked** guest fragment. Prints source and words, including the exceptional self-jump. |
| `execute --op ADD/SUB --a 2 --b 3` | Seed `[2,3]` → `link_guest_add/sub` → `link_guest_arithmetic` → `execute_raw_program` | Linked, stack-aware EVM guest execution. CLI prints status, stack, total count. SUB computes top minus next, here `3 − 2`. |
| `execute --op PUSH1 --value 2` | `execute_guest_push1` → one-element sequence → `compile_push1_program` → `execute_linked_program` | Complete linked executable, words, stack, split counts. |
| `execute --push1-sequence 2,3` | Sequence API → same PUSH1 compiler and executor | Complete linked assembly, stack, split counts. The API takes immediates directly. |
| `execute --op POP/DUP1` | Seed `[0]` → compile bytecode `[0x50]` or `[0x80]` → `execute_linked_program` | Ordinary compiled guest with isolated test inputs. CLI prints result and total. |
| `execute --bytecode HEX` | Parse hex → evaluate both `decode(bytes)` and `execute_guest_evm_bytecode(bytes)` → compile → execute | Strict decoded presentation, guest status, stack, total. A published guest exception is a completed CLI run. A decode or link error fails the command. |
| `decode --bytecode HEX` | `parse_hex` → `disassemble` → `format_disassembly` | Inspection of the original EVM byte stream, including bytes after STOP and missing immediate bytes. No RV64 artifact. |

The redundant decode is real and localized. The tuple in `main.rs` evaluates both operands. The display-side `decode()` calls `decode_detailed()`, and `compile_evm_bytecode()` calls it again. The separate `decode` **command** serves a different purpose. I observed `decode --bytecode 00fe60` list STOP, INVALID, and a truncated PUSH1. Executing `00fe` decoded only STOP and completed normally. Likewise, truncated `60` executes as `PUSH1 0`. A shared decode for execution and its summary should preserve the full-stream disassembler’s distinct behavior. [CLI dispatch](../../crates/ferreth-compiler/src/main.rs) (line 25) · [Double decode](../../crates/ferreth-compiler/src/main.rs) (line 128) · [Strict decoder](../../crates/ferreth-compiler/src/lib.rs) (line 87) · [Disassembler](../../crates/ferreth-compiler/src/disassembler.rs) (line 69)

The whole-program path is:

```text
compile_evm_bytecode(bytes)
  → decode_detailed
  → lower reachable instructions and remember explicit STOP
  → compile_push1_program OR compile_supported_program
  → CompiledRv64Program

execute_guest_evm_bytecode(bytes)
  → execute_guest_evm_bytecode_with_initial_stack(bytes, [])
  → execute_linked_program(compiled artifact, initial stack)
  → execute_raw_program(assembly, bytes, publisher PCs, initial stack)
  → initialize rvemu and guest state
  → run to the selected publisher, then to EXIT_PC
  → validate and retrieve GuestResult and observed instruction counts
```

**`CompiledRv64Program` is sufficient for the current fixed ABI.** It contains final assembly, machine code, publisher entry PCs, and static publisher-count information. Its accessors supply fixed entry and exit PCs and a size-derived execution bound. The initial stack is supplied later. The private `execute_linked_program()` copies the assembly and bytes into the raw execution path. The high-level public bytecode APIs recompile on each call, so a library user has a public compile-only API but no equally convenient public “run this already compiled artifact” API. Lower-level loading and execution remain possible. There is also no whole-bytecode translation command. [Artifact and compiler](../../crates/ferreth-compiler/src/compiler.rs) (line 26) · [Execution boundary](../../crates/ferreth-compiler/src/orchestrator.rs) (line 283)

I found **no direct or indirect execution of generated RV64 in the compilation algorithm**. Assembly, byte patching, reassembly, and layout validation all occur on the host without emulation. The compiler crate depends on rvemu for its execution facilities. REVM is outside its dependency tree. [Compiler manifest](../../crates/ferreth-compiler/Cargo.toml) · [Conformance manifest](../../crates/ferreth-conformance/Cargo.toml)

### Linking inventory and fragment contracts

### Production paths

| Path and callers | Composition and resolution | Publication, validation, and artifact |
|---|---|---|
| **PUSH1-only.** Public `compile_push1_program(immediates)`. Called directly by the sequence/isolated PUSH1 APIs and by the bytecode compiler for all-PUSH1 input without explicit STOP, including empty input. | Generates each body. Sums byte lengths. Places normal, then overflow. Computes each external displacement from the trampoline instruction’s PC. Strips literal assembly prefix/suffix, indexes the entry label, emits a decimal JAL, patches four little-endian bytes. | Local success skips the trampoline and falls through. All failures target one overflow publisher. Reassembles the completed display, compares bytes, validates executable layout, constructs `CompiledRv64Program`. |
| **Supported-program linker.** Private `compile_supported_program`. Called through `compile_evm_bytecode` for reachable ADD/SUB/POP/DUP1 or any explicit STOP. | Dispatches codegen and metadata handling by operation. Places bodies, optional four-byte STOP, normal, overflow, underflow. Applies one or two external patches per fragment and reconstructs indexed assembly with opcode-specific logic. | Fallthrough or explicit STOP reaches normal. Includes both exceptional publishers even if unreachable. Enforces full reassembly equality and layout validation. Constructs the same public artifact. |
| **Isolated arithmetic.** Public `execute_guest_add/sub` and their initial-stack variants. Two thin private adapters call **one** `link_guest_arithmetic`. Used by isolated CLI, guest benchmarks, and tests. | Generates one ordinary guest wrapper and embedded leaf. Validates arithmetic metadata, places normal then underflow, patches the external trampoline and replaces its exact labeled text. An isolated occurrence needs no per-occurrence label renaming. | Builds its own normal and underflow publishers. Checks reassembly and layout. Returns private `LinkedGuestArithmetic` and passes its pieces directly to `execute_raw_program`. |

The two whole-program paths already share publisher text and jump helpers inside `compiler.rs`. The isolated arithmetic linker duplicates the normal/underflow publication instructions, JAL arithmetic, encoding, instruction reading, and final validation pattern. Separate adapters for ADD and SUB do **not** amount to two independent arithmetic linkers. [PUSH1 composition](../../crates/ferreth-compiler/src/compiler.rs) (line 550) · [Mixed composition](../../crates/ferreth-compiler/src/compiler.rs) (line 170) · [Arithmetic composition](../../crates/ferreth-compiler/src/orchestrator.rs) (line 376)

### What the generators actually promise

All offsets below are **RV64 byte offsets within one fragment**.

| Fragment | Local references already resolved | External sites and how the linker finds them |
|---|---|---|
| PUSH1, 44 bytes | Numeric conditional branch reaches the local trampoline. `jal zero, 8` skips it on success. | Overflow at `40`, inferred as `len − 4` by a method on every `GeneratedRv64`. No explicit link metadata. |
| POP, 16 bytes | Symbolic underflow branch resolves inside the fragment. Successful pointer decrement and `jal zero, 8` reach the continuation. | Underflow at `12`, inferred as `len − 4`. Mixed linker checks the original word. |
| DUP1, 64 bytes | Two local conditional branches reach separate trampolines. Successful four-limb copy finishes with `jal zero, 12`. | Underflow at `56`, overflow at `60`, inferred as `len − 8` and `len − 4`. Both original words are checked. Their **meaning** comes from their order. |
| ADD/SUB, 172 bytes each | Nine-word wrapper and 34-word leaf. `jal ra` at `20` calls the embedded core at `36`. `ret` returns to the stack update. The success jump at `28` skips the trampoline and core to `172`. | Generator explicitly supplies `core_call_offset=20`, `exceptional_trampoline_offset=32`, and `normal_continuation_offset=172`. Only the underflow trampoline is externally repatched. |

In the mixed linker, the arithmetic call offset and continuation metadata support validation. In the isolated linker, the continuation **also determines normal-publisher placement**, after validation that it equals fragment length. The call mask checks opcode and destination register `ra`. It does not independently verify the resolved call target. Arithmetic cores introduce genuine **local** control flow and label-collision concerns. They introduce no second external relocation mechanism. [Generated types](../../crates/ferreth-compiler/src/rv64/codegen.rs) (line 13) · [PUSH1](../../crates/ferreth-compiler/src/rv64/opcodes/push.rs) (line 11) · [POP](../../crates/ferreth-compiler/src/rv64/opcodes/pop.rs) (line 17) · [DUP1](../../crates/ferreth-compiler/src/rv64/opcodes/dup.rs) (line 16) · [ADD](../../crates/ferreth-compiler/src/rv64/opcodes/add.rs) (line 19) · [SUB](../../crates/ferreth-compiler/src/rv64/opcodes/sub.rs) (line 19)

`CompiledOperation` identifies lowered operations and carries the PUSH1 immediate. STOP is outside that enum. `GeneratedOperation` identifies already-generated bodies, but also encodes five ways of recovering link information. A bare `GeneratedRv64` does **not** tell a consumer how many external exits it has or which destination each one needs. That knowledge exists across the generator, enum variant, linker arm, and text reconstruction helper. [Lowering](../../crates/ferreth-compiler/src/compiler.rs) (line 112) · [Generated operation variants](../../crates/ferreth-compiler/src/compiler.rs) (line 396)

**Other execution contracts.** `Rv64Runner` assembles the standalone arithmetic leaf, filters display directives, appends an EBREAK sentinel, initializes `a0/a1/a2` and `ra`, and executes until return to that local sentinel. Its result is one U256 and core metrics. `ProgramRunner` is a legacy host-stack arithmetic harness that uses those leaves and rejects POP/DUP1. Neither is an ADR-001 whole guest. Low-level generator tests also run fragments directly, including tests that an unpatched trampoline reaches the instruction limit. Hand-authored metadata-publication tests provide an independent ABI check. These paths have useful purposes outside production guest linking. [Leaf runner](../../crates/ferreth-compiler/src/execution.rs) (line 130) · [Legacy runner](../../crates/ferreth-compiler/src/execution.rs) (line 223) · [Raw fragment test](../../crates/ferreth-compiler/tests/rv64_push1.rs) (line 101) · [Publication tests](../../crates/ferreth-compiler/tests/rv64_guest_result.rs) (line 33)

### Responsibility model

The frontend interprets EVM bytes and immediates, including right-zero-padding truncated PUSH data. Lowering selects supported backend operations; recognition and backend support are distinct. Codegen implements effects and local control flow. Composition places fragments and publishers and binds external transfers. The executor initializes the guest and executes the compiled artifact only when requested. The full-stream EVM disassembler is intentionally separate from execution-prefix decoding. The PUSH1-only linking path is historical compatibility, not a consequence of PUSH immediate decoding.

## 4. Experiments and correctness evidence

### Two consecutive PUSH1s

`translate --op PUSH1 --value 2` printed 11 words. Its final two were `0x0080006f` and `0x0000006f`. The first skips the exceptional slot on success. The second is the unlinked self-jump.

| Component | Relative offset | Calculation |
|---|---:|---|
| First PUSH1 | `0x00` | Entry |
| First trampoline | `0x28` | `10 × 4` |
| Second PUSH1 | `0x2c` | `11 × 4` |
| Second trampoline | `0x54` | `0x2c + 0x28` |
| Normal publisher | `0x58` | `2 × 44` |
| Overflow publisher | `0x74` | `0x58 + 7 × 4` |
| End, exclusive | `0x94` | `0x74 + 8 × 4` |

JAL is relative to the address of the jump instruction. The displacements are `0x74 − 0x28 = 76` and `0x74 − 0x54 = 32`. I observed those decimal operands in the CLI and read **`0x04c0006f` and `0x0200006f`** from the compiled byte array. The sequence API and bytecode API produced equal artifacts across all fields. The executable contains **37 static words**. The successful path executed **27**, comprising ten instructions per PUSH and seven for normal publication. The exit sentinel was excluded. [RISC-V JAL semantics](https://docs.riscv.org/reference/isa/v20260120/unpriv/rv32.html) · [Executor accounting](../../crates/ferreth-compiler/src/rv64/executor/rvemu.rs) (line 118)

### Specimen results

All values in this table were **observed**. Publisher offsets are hexadecimal relative to `ENTRY_PC = 0x80000000`. `N/O/U` designate normal, overflow, and underflow. The dynamic columns count successfully executed RV64 instructions.

| Input | Initial stack | Bytes / static words | Publisher offsets | Guest result | Body + termination = total |
|---|---|---:|---|---|---:|
| Empty bytecode | `[]` | 60 / 15 | N `0`, O `1c` | Normal `[]` | 0 + 7 = **7** |
| `6002` | `[]` | 104 / 26 | N `2c`, O `48` | Normal `[2]` | 10 + 7 = **17** |
| `600200` | `[]` | 140 / 35 | N `30`, O `4c`, U `6c` | Normal `[2]` | 11 + 7 = **18** |
| `60026003` | `[]` | 148 / 37 | N `58`, O `74` | Normal `[2,3]` | 20 + 7 = **27** |
| `600260030100` | `[]` | 356 / 89 | N `108`, O `124`, U `144` | Normal `[5]` | 63 + 7 = **70** |
| `0100` | `[]` | 268 / 67 | N `b0`, O `cc`, U `ec` | Exception 2, `[]` | 3 + 8 = **11** |
| `00` | `[]` | 96 / 24 | N `4`, O `20`, U `40` | Normal `[]` | 1 + 7 = **8** |
| `01`, compiled | `[2,3]` | 264 / 66 | N `ac`, O `c8`, U `e8` | Normal `[5]` | 42 + 7 = **49** |
| Isolated ADD | `[2,3]` | 232 / 58 | Normal and underflow | Normal `[5]` | 42 + 7 = **49** |
| `03`, compiled / isolated SUB | `[2,3]` | 264 / 66 and 232 / 58 | Three and two publishers respectively | Both normal `[1]` | Both 42 + 7 = **49** |

For `600260030100`, the two pushes occupy bytes `0..88`, ADD starts at `0x58`, and its local core starts at `0x7c`. The 172-byte ADD fragment ends at `0x104`. STOP occupies the next four bytes and transfers to normal at `0x108`. The body count is `10 + 10 + 42 + 1`. For `0100`, underflow takes the exceptional trampoline before STOP can execute. Adding STOP to `6002` costs **36 static bytes** in this implementation, comprising a four-byte jump and a newly included 32-byte underflow publisher. Only one extra instruction executes on success.

### Exceptional paths

| Run | Observed result | Dynamic body + termination |
|---|---|---:|
| PUSH1 on a seeded 1024-word stack, both `6002` and `600200` | Exception 1. All 1024 original words preserved. `final_s1 = 0x80109000`. | 4 + 8 |
| DUP1 on the full stack | Exception 1. All words and `s1` preserved. | 5 + 8 |
| DUP1 or POP on empty stack | Exception 2. Empty stack, `final_s1 = 0x80101000`. | 2 + 8 |
| ADD on zero or one word | Exception 2. Original stack and `s1` preserved. | 3 + 8 |

The isolated CLI forms of POP and DUP1 succeeded with their seeded `[0]` inputs, yielding `[]` in 10 and `[0,0]` in 21 instructions. That input difference accounts for their behavior relative to empty-stack bytecode execution. None of the Ferreth counts above measures EVM gas.

### Reproduced compilation defect — valid forward jump rejected

| Repeated input, empty entry stack | Earliest overflow JAL displacement | Ferreth observation |
|---|---:|---|
| `6002` × 46, EOF | 2012 | Normal, 46 twos, 467 RV64 instructions |
| `6002` × 46, then `00` | 2016 | Normal, 46 twos, 468 RV64 instructions |
| `6002` × 47, EOF | 2056 | Compile/link error `ImmediateOverflow { value: 2056, min: -2048, max: 2047, … }` |
| `6002` × 47, then `00` | 2060 | Same error, `value: 2060` |

For `n` pushes without STOP, the first displacement is `44n + 28 − 40`. At 47 pushes, the expected executable is only 2,128 bytes and maximal stack depth is 47. The displacement fits the ISA and Ferreth’s code reservation. With frame budget 141, **EELS and direct REVM both completed both 47-PUSH programs**, stacked 47 twos, and ended with zero gas.

The cause is specific. Ferreth’s binary path accepts these forward offsets. Its final display contains numeric `jal zero, 2056` or `2060`. In `asm-rs 0.2.0`, numeric JAL goes through the signed 12-bit immediate reader, while label JAL uses `RvJal20`. I independently assembled a symbolic jump from `0x80000000` to `0x80000808`. The assembler emitted `0x0090006f`, reported the target address, and returned an applied `RvJal20` relocation at byte zero. This is a **compile-time rejection of valid supported input**. No wrong guest result was returned. [Compiler reassembly](../../crates/ferreth-compiler/src/compiler.rs) (line 598) · Pinned numeric operand reader (`asm-rs-0.2.0/src/riscv.rs:439` in Cargo registry) · Pinned JAL dispatch (`asm-rs-0.2.0/src/riscv.rs:1982` in Cargo registry)

### Classification of failures

- LF-026 is a present compiler correctness defect: valid supported input is rejected before execution. No silently wrong guest result was observed.
- LF-027 is a verified helper defect with latent production impact: current publisher patches are forward, but a signed backward jump is encoded incorrectly.
- LF-031 is an independent benchmark output defect, not an EVM semantic failure.

The 47-PUSH example is especially relevant before gas: adding inline charging increases fragment lengths and can reach the accidental numeric limit sooner. Do not remove assembly validation merely to make the program compile.

## 5. Consolidated findings and problem statement

In this table `compiler`, `orchestrator`, and `codegen` refer to the source files linked in sections 3–4. Line numbers refer to the recorded revision.

| ID | Verdict | Evidence and qualification |
|---|---|---|
| LF-001 | **Confirmed** | `main:25,58,209` expose leaf source, unlinked fragment, linked guest execution, and EVM inspection. Several differences serve distinct contracts. |
| LF-002 | **Confirmed** | `compiler:112` ends in an artifact. `orchestrator:297,304,316` accepts the artifact, then initializes and runs the guest. The artifact runner is private. |
| LF-003 | **Confirmed** | Compiler inputs have no stack parameter. Running `01` with zero, one, or two words changes outcomes while the returned executable bytes match separately compiled bytes. |
| LF-004 | **Confirmed** | `compiler:136` selects PUSH1-only for all reachable operations being PUSH1 and no explicit STOP. Empty input qualifies vacuously. Bytes after the first STOP never enter selection. Direct sequence callers bypass selection. |
| LF-005 | **Confirmed** | The strict decoder groups PUSH1–PUSH32. `compiler:122` specifically matches `opcode: 0x60` and rejects other decoded PUSH widths with their byte offsets. |
| LF-006 | **Confirmed** | `compiler:550` generates all bodies, lays out publishers, patches, rebuilds source, validates, and constructs the artifact. |
| LF-007 | **Partially confirmed** | Current production linking can fail on formatting because reconstruction uses exact prefixes, suffixes, indentation and instruction strings. Binary displacement arithmetic itself does not inspect whitespace. Some whitespace changes are harmless. |
| LF-008 | **Confirmed** | The comparison appears at `compiler:379,598` and `orchestrator:473`. A reassembly error also prevents artifact return, as the 47-PUSH result demonstrates. |
| LF-009 | **Confirmed** | Arithmetic has optional explicit metadata. PUSH1, POP, and DUP1 rely on trailing positions and conventions. Destination identity remains implicit even where an offset is explicit. |
| LF-010 | **Confirmed** | `codegen:76` returns `machine_code.len() − 4` on the general result type. Its overflow interpretation is valid for PUSH1, but the method is also callable on an ADD ending in the core’s `ret`. |
| LF-011 | **Confirmed** | Both displacement calculations, encoded words, artifact size, final stack, and split dynamic counts were independently reproduced. |
| LF-012 | **Confirmed** | `compiler:174` collects operation-specific metadata. `compiler:277` dispatches again for placement-dependent source and binary changes. |
| LF-013 | **Confirmed** | `compiler:180,198` repeat the continuation-length, masked core-call, and exact trampoline-word checks. Isolated arithmetic already shares these checks across ADD/SUB. |
| LF-014 | **Confirmed** | `compiler:216,231` derives POP’s last slot and DUP1’s last two slots. Checking identical self-jump words cannot by itself distinguish DUP1’s underflow and overflow meanings. |
| LF-015 | **Confirmed** | Arithmetic, POP, and DUP1 check selected original words. PUSH1 has generator width and textual checks but lacks an equivalent prepatch word check in the production linkers. |
| LF-016 | **Confirmed** | Both compiler paths own placement, patching, reconstruction, final assembly and construction. Isolated arithmetic repeats much of that work and the publication bodies. |
| LF-017 | **Confirmed** | The mixed linker repeatedly computes site PC, displacement, encoding, byte replacement and append. DUP1 does it twice. Current external sites all use the same machine instruction. |
| LF-018 | **Confirmed** | `has_explicit_stop` is separate from `CompiledOperation`. It selects the mixed path, allocates one word, shifts publisher addresses, and emits `jal zero, 4`. |
| LF-019 | **Partially confirmed** | The useful contracts are evident: PC-relative target arithmetic, explicit word checking, publication, layout validation, and final equivalence. Preserving every current implementation would also preserve a signed-encoding defect described below. |
| LF-020 | **Confirmed** | All unresolved exceptional trampolines examined start as `0x0000006f` and are changed to JAL with `rd=x0`. The preceding local branches and arithmetic core calls have distinct roles. |
| LF-021 | **Partially confirmed** | The enum demonstrably carries opcode identity and heterogeneous link data. Its historical motivation is an inference. The lowered operation also has a legitimate codegen-dispatch role. |
| LF-022 | **Confirmed** | Both whole-program linkers require exact byte-vector equality. The isolated arithmetic linker provides the same check. Equality establishes consistency between representations, not the semantic correctness of the selected target. |
| LF-023 | **Confirmed** | PUSH1 uses inline prefix/suffix replacement. POP, DUP1, ADD and SUB have separate reconstruction helpers at `compiler:431–543`. |
| LF-024 | **Confirmed** | The linker knows `guest_*_underflow`, `evm_add/sub`, `jal ra`, `bltu s1, t0`, `bgeu s0, s1`, exact label newlines, indentation, and sometimes expected replacement counts. |
| LF-025 | **Confirmed, static scope** | `compiler:390` uses overflow publisher length as the single exceptional count. Both exceptional publishers are eight words today. Runtime metrics use two observed executor reports and remain valid if publisher lengths differ. |

[Lowering and metadata validation](../../crates/ferreth-compiler/src/compiler.rs) (line 112) · [Mixed patching](../../crates/ferreth-compiler/src/compiler.rs) (line 277) · [Text reconstruction](../../crates/ferreth-compiler/src/compiler.rs) (line 431) · [Observed accounting](../../crates/ferreth-compiler/src/orchestrator.rs) (line 343)

### Additional findings from independent review

| ID | Classification | Finding and implication |
|---|---|---|
| **LF-026** | **Observed, current compiler defect** | The 46/47 boundary affects both whole-program paths. The current seeded differential generator caps bytecode at 64 bytes. The 94- and 95-byte witnesses lie outside that corpus. They warrant directed regression cases. [Generator limits](../../crates/ferreth-conformance/tests/rc_revm_conformance.rs) (line 311) |
| **LF-027** | **Observed helper defect, latent in production** | Both copies of `encode_jal_zero` accept signed offsets but omit immediate bit 20, which belongs in instruction bit 31. I evaluated the two exact function bodies from this revision at `−4`. Both produced `0x7fdff06f`; the assembler produced `0xffdff06f`. The former encodes `+1048572`. All current production publisher patches point forward, so I found no current guest path taking that wrong jump. [Compiler encoder](../../crates/ferreth-compiler/src/compiler.rs) (line 692) · [Orchestrator encoder](../../crates/ferreth-compiler/src/orchestrator.rs) (line 542) |
| **LF-028** | **Observed and source** | The linked-artifact boundary is stronger internally than the public execution API suggests. Public isolated arithmetic can use ordinary bytecode compilation with the same seeded stack and observed metrics. Artifact text, exceptional displacement, publisher inventory and static size can change. [Isolated APIs](../../crates/ferreth-compiler/src/orchestrator.rs) (line 205) |
| **LF-029** | **Source, future constraint** | The automatic execution limit is static code length divided by four. It works for the current emitted paths, including locally called cores each executed at most once per occurrence. A repeatedly called shared gas routine or future EVM loop would invalidate that general bound. Inline gas charging fits the current assumption. [Artifact bound](../../crates/ferreth-compiler/src/compiler.rs) (line 53) · [Runtime bound](../../crates/ferreth-compiler/src/orchestrator.rs) (line 343) |
| **LF-030** | **Observed assembler capability** | The pinned builder returns final bytes, absolute named-label addresses and **applied** relocations with output-relative patch offsets. The Phase 1.1 forward JAL test records the pinned API and verifies the result at a nonzero origin. Earlier probes also resolved backward label JALs, accepted a predefined external address, and rejected duplicate and undefined labels. Ferreth still needs independent checks of intended destinations before production integration. [Phase 1.1 evidence and regression](../../research/guest-linking/phase-1/results/1.1-assembler-api.md) |
| **LF-031** | **Observed, separate CLI output defect** | With `dtc` unavailable, both benchmark commands exited successfully but raw stdout was invalid JSON. rvemu printed four warning lines before the core report and thirty before the guest report. Stderr was empty. Parsing the JSON suffix yielded the expected measurements. This affects reproducible evidence capture in this environment and is independent of guest semantics. [Benchmark CLI](../../crates/ferreth-conformance/src/main.rs) (line 70) · rvemu warning source (`rvemu-0.0.11/src/rom.rs:138` in Cargo registry) |
| **LF-032** | **Source, ABI coupling** | Publishers derive metadata from `s0 − 4096`, use literal field offsets, and jump to metadata base minus four. This agrees with today’s memory layout. Duplicating that knowledge in isolated and compiler-owned publishers creates a concrete second edit site for ADR-002. [Compiler publishers](../../crates/ferreth-compiler/src/compiler.rs) (line 614) · [Isolated publishers](../../crates/ferreth-compiler/src/orchestrator.rs) (line 489) |

**Fix-worthy now.** LF-026 prevents compiling a small, valid program built entirely from supported instructions. The exact-string reconstruction and duplicated publisher ownership also have a demonstrated engineering cost: the former participates in the failure, and the latter gives a gas ABI change two independently maintained implementations. LF-027 rules out promoting the current JAL encoder unchanged to a general signed primitive.

**Acceptable today.** Repeated arithmetic cores preserve the tested leaf implementation. Local stack checks protect ADR-001 invariants. Success jumps over trailing trampolines, unused publishers, and larger artifacts are compatible with the project’s baseline. Spending a refactor on dead publisher elimination or arithmetic-core deduplication would add no demonstrated semantic value here. Dynamic termination measurement is already sound for the tested programs.

**Historical compatibility was deliberate.** `9ac51eb` introduced sequence compilation. `e119444` routed bytecode through it and tested equality of source and bytes between the two public inputs. When `ded83c5` introduced mixed ADD compilation, its diff explicitly retained the established PUSH1-only linker for exact output to existing callers. STOP subsequently acquired its own transfer. POP and DUP1 later demonstrated that isolated execution can delegate to RC. The current test compares **two current API results**. Both could change together and still pass. It is evidence of cross-entry equality, not an immutable historical golden file. The arithmetic-core benchmark has a stronger documented byte-identity expectation through its hashes. [Current PUSH equality test](../../crates/ferreth-compiler/tests/rv64_push1_program.rs) (line 304) · [Explicit STOP text assertion](../../crates/ferreth-compiler/tests/rv64_push1_program.rs) (line 164) · [Core benchmark contract](../../docs/runbooks/benchmark.md) (line 53)

I found **no silent wrong EVM result attributable to the duplicate linkers** in the exercised guests. The long-jump rejection, latent signed-encoding error, and benchmark stdout problem have different failure modes and should be tracked separately.

The primary problem is the cost of introducing and verifying semantic changes across the compiler, not the existence of multiple functions or the current instruction count. In particular, the same JAL-zero external mechanism is expressed with per-opcode conventions and repeated binary and textual reconstruction. Consolidating only the outer entry points would leave the latter problem intact. Similarly, extracting the existing encoder unchanged would centralize a defect rather than fix it.

## 6. Gas metering as an architectural stress test

ADR-002 is **proposed**. Its immediate milestone covers the existing six operations plus GAS under Cancun. It proposes an explicit `u64` frame budget, guest-owned mutable gas, charge before effects, zero terminal gas on exceptional halt, and publication before the existing sentinel. The following is an impact assessment. [Full proposal](../../docs/adr/002-cancun-evm-gas-metering.md) (line 42)

| Requirement | Ownership | Impact on the present implementation |
|---|---|---|
| Initial frame budget | Execution inputs and guest ABI | Pass the budget alongside initial stack to the common execution setup. Initialize the input limit and live remaining gas in guest memory. Extend whole-program, sequence and isolated guest entry points and their benchmark/test callers. The same executable should accept different budgets. |
| Live and terminal gas | Guest code and ABI | Extend metadata encoding, initialization, result retrieval and validation. Preserve the 24-byte prefix, terminal status values, `s0/s1`, and status-last publication. Validate remaining against the supplied limit and derive used gas with checked subtraction. The proposal leaves exact extension details for review. |
| Per-opcode charge | Lowering/codegen | Emit one unsigned sufficient-gas check and one debit on success before opcode effects. Costs are PUSH1/ADD/SUB/DUP1 `3`, POP/GAS `2`, STOP `0`. Meter arithmetic in its guest wrapper, preserving the pointer leaf. A common emitter can generate inline instructions without adding a runtime calling convention. |
| OOG detection and routing | Codegen, linker, guest publisher | Codegen needs a failure transfer that skips the opcode’s effects. Composition needs a distinguishable OOG destination. Today PUSH1 has one implied exit, arithmetic metadata describes one exception, and DUP1 has two positional exits. All three composition paths would need to accommodate OOG or delegate. |
| Exceptional gas settlement | Guest publishers | Every OOG, stack-underflow and stack-overflow terminal path must publish zero remaining before status. Normal publication preserves actual remaining gas. The two compiler paths share publisher source, but compose separate inventories. Isolated arithmetic has its own publisher source and inventory. |
| Stack-exception ordering | Guest wrappers and regression policy | Below cost, proposed Ferreth diagnostics select OOG before stack checks. At sufficient gas, the operation may charge and then raise its stack error. Preserve live stack and `s1` on failed checks. Cross-engine comparisons need to tolerate documented diagnostic differences. |
| Explicit STOP | Lowering and normal publisher | The flag currently emits one body instruction. Zero cost must permit STOP at budget zero and preserve unused gas. No OOG site or debit is necessary for a known zero cost. Retain termination before trailing bytes. |
| Implicit end of bytecode | Composition and normal publisher | Fallthrough, including empty input, must preserve remaining gas without adding a synthetic paid operation. Both PUSH1-only and mixed fallthrough require this behavior. |
| GAS `0x5a` | Decoder, lowering, codegen, linker, ABI | Recognize in strict compilation, charge 2, check capacity, push the **post-charge** `u64` as a zero-extended U256. Request OOG and overflow exits. The disassembler knowing the mnemonic does not provide backend support. |
| Isolated versus core execution | API adapters and benchmarks | Apply the complete frame contract to seeded ADD/SUB/POP/DUP1 and PUSH1/sequence guests. Keep the `opcode_core` pointer benchmark and legacy arithmetic harness outside that frame contract. Host faults and instruction-limit exhaustion remain host errors. |

Two independently rerun vectors matter to the ADR review:

| Program and frame gas | EELS observation | Direct REVM observation | Interpretation |
|---|---|---|---|
| `60020100`, 20 | Underflow, remaining `0`, stack `[]`, PC `2` | Underflow, raw remaining `14`, stack `[2]`, PC `3` | REVM handler source settles exceptional frame gas to zero. Preserve raw `14` in research evidence. Compare Ferreth’s guest-published result to settled `0`. |
| `0100`, 2 | Stack underflow, remaining `0`, PC `0` | OOG, raw remaining `0`, PC `1` | Charge-first is a possible deterministic **Ferreth diagnostic policy**. ADR-002’s claim of uniform specification ordering is too broad. Pinned EELS ADD pops before charging. |
| `8000`, 3 | Stack underflow, remaining `0` | Stack overflow, raw remaining `0` | Keep the existing distinct REVM/Ferreth diagnostic regression. Both classify as exceptional. |
| `5a5a00`, 10 | Normal `[8,6]`, remaining `6` | Normal `[8,6]`, raw remaining `6` | Both establish guest-observable charge-before-read for GAS. |

The direct REVM runner now supplies practical evidence for the proposal’s frame-equivalence question. The ordinary conformance suite still uses a transaction gas limit of `1_000_000`. A production-quality frame comparator remains pending. REVM settlement above is source-derived from its frame/handler path; the direct runner measured raw interpreter gas. REVERT must retain its separate unused-gas treatment when supported. [RF-002 and RF-003](../../docs/research/ferreth-evm-conformance.md) (line 346) · [Direct frame runner](../../research/revm-frame/src/main.rs) (line 58) · [Transaction harness](../../crates/ferreth-conformance/tests/rc_revm_conformance.rs) (line 50) · REVM settlement (`revm-handler-43.0.3/src/frame.rs:595` in Cargo registry)

**Answers to the seven architectural questions**

1. **More opcode-specific branches?** Under the current pattern, yes. Each affected arm must locate another site, select OOG, patch bytes and repair text. One declared destination per external site eliminates the need for a distinct OOG linking algorithm per opcode.
2. **Duplicated settlement changes?** Yes. Normal and exceptional publisher instructions have two owners, and three composition paths determine which publishers exist and where they begin.
3. **Risk of different gas semantics?** Yes. Isolated ADD could debit correctly but use an older publisher, or the sequence path could omit OOG while mixed PUSH1 handles it. Shared opcode bodies alone cannot prevent either inconsistency.
4. **Adequate fragment contract?** No. It cannot describe the existing DUP1 exits uniformly, much less add OOG without a new convention. The required extension is small because all current external sites use JAL-zero.
5. **Responsibility split?** The ABI defines fields, ordering and diagnostics. Codegen owns charging and effect order. Composition resolves transfers and includes publishers. Guest publisher code settles gas. Execution initializes and validates the frame and observes its result.
6. **Gas first on the present architecture?** A narrowly tested implementation is possible after ADR approval. I would decline that sequence as the baseline. Larger metered bodies would reach the demonstrated numeric-JAL limit sooner, while adding exit kinds increases duplicate integration work.
7. **Smallest useful preparation?** Make a publisher reference independent of mnemonic and decimal displacement, establish a single owner for guest composition/publication, and route isolated arithmetic to it. That directly removes the demonstrated failure mechanism and the second settlement implementation.

The goal is one owner for exceptional settlement and one reusable means of requesting external transfers. Per-opcode costs, stack checks and effect ordering remain legitimate semantic codegen work. Inline charging is a reasonable initial approach under the current execution bound; sharing a repeatedly invoked gas subroutine is not required and would require revisiting that bound. ADR-002 remains proposed, including its exact ABI layout and diagnostic precedence. Do not implement it during linking preparation.

## 7. Alternatives and maintainer direction (to-be)

### Alternatives

| Alternative | Judgment |
|---|---|
| **A — Preserve current architecture** | Credible control for short inputs. Lowest migration cost and excellent compatibility. Adding gas requires touching every appropriate link path and maintaining both publisher implementations. Preserving LF-026 disqualifies literal A as a reliable baseline. |
| **B — Share linking primitives** | Central validation, publication and JAL handling would contain genuine risk, especially LF-027 and settlement drift. Merely extracting byte writes saves lines while leaving every opcode arm responsible for choosing a destination and repairing its text. B is valuable as an intermediate step. |
| **C — Unified guest linker** | Feasible with current APIs. POP/DUP1 already delegate, and seeded arithmetic parity was observed. PUSH1-only can enter the same composer through an immediate adapter. Retaining mixed opcode-specific patch arms still makes OOG integration repetitive. A naive redirect to today’s mixed layout appends an underflow publisher to PUSH1-only output. |
| **D — Minimal uniform fragment contract** | The evidence supports a short list of external exits with a site and semantic target. JAL-zero can be implicit while it is the only mechanism. Local branches, embedded calls and continuations can stay generator-owned. D reduces linker edits per opcode, but multiple composers could still publish different gas semantics. |
| **E — Staged consolidation** | First establish compatibility and long-jump regressions, then normalize external references and converge guest APIs. Preserve the legacy path as a comparison during review. This permits a focused equivalence check before gas changes intentionally alter guest artifacts. |
| **F — Compose symbolic assembly and use the pinned assembler as layout authority** | A serious simpler implementation of the shared composer. Generators emit uniquely named local labels and references to a small fixed set of publisher names. The assembler resolves control flow and supplies final publisher PCs. It can avoid a permanent handwritten encoder and binary-relocation layer. It requires independent checks on final targets and a decision on numeric display compatibility. |

### Assembly strategy is a separate decision

| Strategy | What it guarantees | Limits and compatibility cost |
|---|---|---|
| Generalize the existing string rewrites | A bounded change can retain legacy text and the current final byte comparison. | Finding labels or instructions by register spelling and whitespace retains LF-007/LF-024. Decimal external operands retain LF-026. A more elaborate string matcher is poor preparation for gas. |
| Generator-owned unique local labels | A deterministic occurrence identifier can namespace wrapper, branch and core labels at emission. Local work remains independent of total program length. | Solves collisions, but does not by itself bind external exits. An isolated fragment can retain its familiar labels through its existing API. Exact display fixtures may change for composed guests. |
| Structured patch information for text and bytes | A generator can identify an aligned patch word, a source anchor and an intended publisher. One iterator can verify the original word, patch bytes, render a symbolic target, and compare full reassembly. | Maintains two representations, but gives a strong independent cross-check during migration. A real anchor must survive formatting changes. Current `GeneratedRv64LinkMetadata` is insufficient. A new patch-kind hierarchy is unnecessary for one kind. |
| Whole symbolic source, assemble once | The readable source is the source of the loaded bytes. The builder supplies label addresses and applied relocations. Local names avoid collisions, publisher names express external intent. The far-jump experiment succeeds. | Reassembling the same source alone is a weak independent test. Decode final JAL words and check their destinations, publisher PCs, local calls and continuations independently. Symbolic rendering changes previously printed decimal operands even when machine bytes match. |
| Derive assembly from final bytes | A suitable disassembler could inspect exactly what would execute. | The repository disassembler consumes **EVM** bytes. I found no existing public RV64 binary-to-readable-source facility in the active compiler dependencies that also recovers Ferreth’s labels and publisher roles. The assembler listing uses source annotations. A new dependency or handwritten disassembler has higher scope and loses names without extra metadata. |

**Pinned assembler limits matter.** `assemble()` finishes a complete assembly and returns bytes. The builder accepts forward references, exposes final labels and applied relocations, and supports external symbols **at known addresses**. Undeclared targets and duplicate definitions failed in my probe. This does not establish a supported unresolved-object workflow. The builder resolved symbolic JALs at both signed ISA boundaries and rejected an odd target and out-of-range displacements. Ferreth’s fixed-width executable contract additionally requires four-byte instruction alignment. Assembler API (`asm-rs-0.2.0/src/lib.rs:134` in Cargo registry) · Applied relocations (`asm-rs-0.2.0/src/linker.rs:192` in Cargo registry)

I also measured conditional-branch relaxation. A conditional branch over a 100-byte gap used one word. Over a 5,000-byte gap it expanded to two words, with a JAL relocation in the second. This argues for keeping the short local branch to a nearby trampoline, or deriving addresses from final assembly wherever relaxation is permitted. It also rules out calculating publisher PCs by counting source lines. No change to core arithmetic or historical core hashes is necessary under any of the viable assembly strategies. Pinned RV64 relaxation (`asm-rs-0.2.0/src/riscv.rs:1834` in Cargo registry)

Scores are anticipated suitability, 1–5. Each cell gives its reason. Totals use the requested weights. For B–F, the score assumes the candidate explicitly resolves LF-026 and preserves independent validation.

| Alternative | Correctness 30% | Gas risk 25% | Expansion 15% | Verification 15% | Scope 10% | Compatibility 5% | Total / 5 |
|---|---|---|---|---|---|---|---:|
| A | **3** short cases pass, long valid input fails | **2** three integrations | **1** new opcode arms accumulate | **2** each path needs separate proof | **5** minimal change | **5** existing output | **2.60** |
| B | **4** one checked patch/publish primitive | **3** shared settlement, separate routing | **2** per-op contracts persist | **4** helpers test independently | **4** bounded extraction | **5** layout can stay identical | **3.50** |
| C | **4** shared guest compiler, migration risk | **4** one composition policy | **3** mixed patch dispatch remains | **4** RO/RC parity directly testable | **4** adapters already demonstrated | **4** static layouts can differ | **3.85** |
| D | **4** explicit sites and destinations | **3** publisher ownership may remain split | **5** existing mechanism reusable | **5** site/target checks are direct | **3** generator and display changes | **4** byte identity feasible | **3.95** |
| E | **5** old/new comparisons at each stage | **5** one settlement owner before gas | **4** remaining dispatch belongs mainly to codegen | **5** differential and artifact gates | **3** several bounded changes | **4** exact-output decision isolated | **4.60** |
| F | **4** assembler resolves targets, migration needs checks | **4** shared named exits | **5** new references use existing assembler | **4** final words/labels inspectable, same-assembler round-trip weak | **4** little bespoke linker machinery | **3** historical numeric display changes | **4.10** |

**Correctness gate.** Literal A fails on LF-026. Extracting its defective encoder unchanged would also fail the gate for B. A symbolic composer that accepts whatever its assembler returns without checking intended destinations would fail the independent-verification requirement. E is my recommendation because the migration can prove the two facts that matter most: equivalent guest behavior across entry points, and correct control transfers beyond the current accidental range limit.

**Confidence.** High for the traced ownership, length defect, signed-encoding defect, observed ABI behavior, and pinned assembler capabilities. Moderate for relative implementation effort and future compatibility. Upstream consumers of the exact printable PUSH1 text have not been inventoried. F could be less work than D if that text can change. E and F are compatible: E describes migration order, F a possible final implementation.

### Candidate acceptance principles

| Principles | Evaluation across alternatives |
|---|---|
| **1–3. Compile without execution, execute artifacts, use shared semantics** | All alternatives can preserve 1. A/B retain an additional private arithmetic artifact and require separate parity checks for 2–3. C/E/F naturally unify equivalent guest paths. D alone leaves entry routing open. Apply 3 within an equivalent ABI and input contract. |
| **4–5. Program-length-independent codegen and no linker edit for a familiar mechanism** | Local codegen may legitimately use **fragment** length, as ADD does to skip its core. It need not know whole-program length. A/B/C retain type-specific linker work to varying degrees. D/E/F can meet 5 when an opcode uses known publisher targets. Adding a genuinely new termination category still requires declaring its semantics. |
| **6. Cross-cutting semantics have one owner** | A fares poorly. B helps if publication is shared. C gives settlement one owner but can retain repeated patch arms. D gives references one form but can retain competing publishers. E/F can address both. Per-opcode costs and stack preconditions remain legitimate opcode-specific work. |
| **7–8. Explicit validation and assembly/byte agreement** | A–D can retain today’s two-stream comparison. E should preserve it during migration. F needs independent target decoding and ABI outcome tests in addition to source/byte consistency. LF-026 demonstrates why keeping readable output matters even when binary arithmetic is correct. |
| **9–10. Preserve coverage and defer optimization** | Feasible for all candidates. Keep the compiler deterministic suite, transaction REVM differential, low-level ABI tests, and three benchmark scopes. None of the alternatives needs faster code, fewer cores, or smaller guests to pass. |

### Review of the decision matrix

The matrix is the independent researcher's prospective evaluation, not an implemented comparison. Alternative E is a migration strategy rather than an exclusive architecture; F is a candidate implementation and D a possible contract. They can coexist. The correctness gate has priority over numerical scores. Literal A fails the known supported-input regression; a fixed version of A would need separate evaluation if the symbolic spike fails.

### Maintainer direction, 2026-11-19

- Prefer symbolic assembly and verified capabilities of `asm-rs 0.2.0` rather than maintaining exact rendered text and an unnecessary second linking implementation.
- Prioritize semantic equivalence, accepted ABI behavior and independent verification for guest programs. Changes in guest bytes and metrics must be explained; do not assume external compatibility obligations have been inventoried. Preserve documented standalone arithmetic core hashes and benchmark contracts unless separately reviewed.
- Keep `CompiledRv64Program`, existing arithmetic cores, and the emulator execution boundary. No emulator move, canonical opcode redesign, general object format, compiler IR, or optimization is required for this milestone.
- Investigate a minimal uniform fragment contract only to the extent needed to express actual external targets and validate their binding. Generator-owned local labels and symbolic publisher names may suffice.

Candidate architecture (not yet implemented):

```text
EVM bytecode -> decode -> lower -> opcode generators
                                    |
                        symbolic guest fragments
                                    |
                one guest composer + shared publishers
                                    |
                     asm-rs resolves and emits bytes
                                    |
                    CompiledRv64Program + metadata
                                    |
                   optional execution with initial state
```

The composer owns layout policy and publisher inventory; assembler-produced label addresses establish actual placement. The generator owns local control flow and requests semantic exit targets. The executor does not decide translation semantics. A new opcode using existing linking mechanisms should not require changes to the linking algorithm. A new termination category can legitimately require defining its publisher and semantics.

**A stronger acceptance principle:** Ferreth must provide one verifiable mechanism for composing EVM guests and resolving their external transfers. Cross-cutting semantic requirements, including gas and exceptional settlement, must have explicit ownership without duplicated opcode-specific linking algorithms.

## 8. Saturday plan — staged preparation, not a deadline

Each phase has an independent stop point. Review evidence before advancing. Do not assume that all phases must fit into one session. Use a dedicated branch only when implementation is approved; keep each change reviewable. The following is a proposed work plan, not authorization to modify code without maintainer review.

### Phase 0 — Reproduce and protect

- Start with `AGENTS.md`, this document, accepted ADR-001 and proposed ADR-002; check branch, status and revision.
- Reconcile the prior log's 136 reported tests with the independent run's 135; inspect discovery and expected differential case counts, not just exit status. Capture current compiler and conformance results, specimen machine words, publisher PCs, guest status/stack/s1, static/dynamic counts and standalone core hashes.
- Reproduce the 46/47 PUSH boundary with and without STOP. Add directed failing regressions when authorized, including a taken distant overflow jump on a full seeded stack. Reproduce the signed JAL helper defect independently.
- Record compatibility contracts: guest text is not frozen, guest semantic equivalence is required, and standalone core byte identity is protected. Review APIs or fixtures with external consumers before removal.

**Exit gate:** Known failure modes and baseline are reproducible; expected test discovery is understood; no unrelated changes. A deliberately failing regression is evidence, not a green integration checkpoint.

### Phase 1 — Symbolic assembly spike

Phase 1.0 is approved and Phase 1.1 met its exit gate. Phase 1.2 awaits
maintainer authorization. [Incremental plan](../../research/guest-linking/phase-1/PLAN.md)

- In a bounded experiment, compose repeated PUSH1 and representative ADD/SUB/POP/DUP1 fragments with uniquely named local labels and symbolic external publisher targets. Keep short local conditionals and embedded calls generator-owned.
- Use the pinned assembler's result API to obtain final bytes and label addresses. Verify forward and backward JAL, alignment and range rejection, duplicate/undefined labels, and the effect of branch relaxation. Never infer final addresses from source line counts.
- Independently decode final jump words and compare actual targets to intended publisher PCs and local continuation/core targets. Verify 46/47 specimens and actually taken distant exception. Compare short guest semantics and metrics against baseline.
- Determine whether symbolic assembly alone suffices or a small explicit reference manifest is needed to validate intent. Do not replace the current composer during the spike.

**Exit gate:** Demonstrated support for needed references, artifact metadata and independent target verification. If the spike fails, document the limiting capability and evaluate the smallest checked patching alternative; do not invent a universal linker.

### Phase 2 — Consolidate guest composition

The [Phase 2.3a fragment contract](../../research/guest-linking/phase-2.3a-fragment-contract.md)
supports private symbolic PUSH1/STOP and the
[Phase 2.3b POP/DUP1 fragments](../../research/guest-linking/phase-2.3b-pop-dup1.md).
[Phase 2.3b.1 boundary evidence](../../research/guest-linking/phase-2.3b.1-boundary-hardening.md)
closes the oversized DUP1 JAL question and verifies directed runtime parity.
The maintainer accepted Phase 2.3b.1, closed RF-019 and recorded RF-020 as
a known implementation capacity limit. The default assembler limits and
the ADR-001 executable bound apply independently.
The [Phase 2.3c report](../../research/guest-linking/phase-2.3c-symbolic-arithmetic.md)
records RF-022 and the private ADD/SUB integration, including exact leaf
identity, fixed-layout call/return verification and directed runtime parity.
The [Phase 2.3d integration review](../../research/guest-linking/phase-2.3d-integration-review.md)
records corrected unsupported-PUSH error precedence (RF-023), required publisher
PC validation (RF-024), and declaration-site ownership (RF-025). All six private
opcodes passed the gate. The isolated arithmetic singleton images already match
in full. Public compilation still reproduces the four LF-026 controls.
The [bounded Phase 2.4 plan](../../research/guest-linking/phase-2.4-cutover-plan.md)
awaits maintainer review of public artifact metadata and routing.

- Implement only the reviewed mechanism from Phase 1. Give one composer ownership of fragment placement, external transfers and publisher composition. Keep one consistent publication implementation for all applicable guests.
- Route bytecode and sequence compilation through that composer. Route isolated ADD/SUB through ordinary single-op guest compilation with seeded initial stack, following POP/DUP1. Preserve useful convenience APIs as thin adapters where possible.
- Remove redundant text reconstruction and handwritten encoders only after parity is established. If an encoder remains, correct and test signed immediate encoding. Preserve independent verification of targets, emitted layout, publisher entry PCs and readable assembly.
- Keep standalone pointer-ABI arithmetic execution separate. Do not add gas, new opcodes, or unrelated CLI or decoder changes in this phase.

**Exit gate:** One guest composition/publication owner; no opcode-specific linking algorithm for existing external transfer mechanisms; long-PUSH regressions pass on the new path; old/new comparisons reviewed before deleting redundant paths.

### Phase 3 — Correctness and integration gate

- Run the complete compiler and conformance suites with locked dependencies; verify test discovery and seeded coverage. Exercise short specimens, repeated label occurrences, explicit STOP and EOF, empty and seeded stacks, underflow and overflow paths, and both DUP1 exits.
- Independently check branch targets, code bounds/alignment, status-last publication, guest ABI invariants, sentinel exclusion and host-error separation. Confirm semantic parity across equivalent entry points and unchanged standalone core hashes. Explain intentional differences in guest bytes, static sizes or dynamic counts.
- Review the diff, formatting, lints, benchmark output limitations and documentation. Seek independent cross-review before merging. Stop once the floor is correct and understandable; no performance optimization is needed.

**Exit gate:** All required regressions and existing conformance pass, correct external binding is independently evidenced, and publisher ownership is unambiguous. The maintainer reviews and approves integration before treating this milestone as complete.

### Next milestone — ADR-002 Cancun gas metering

Only after the linking preparation gate, review ADR-002 against RF-002/RF-003/RF-007 and the new architecture. Approve exact gas ABI, charge ordering/diagnostic policy, exceptional settlement and host/guest separation before implementation. Reuse the chunk plan from the November 18 engineering log: ABI transport, inline metering primitive, settlement, incremental metering of existing opcodes, GAS, frame-level differential conformance, final integration. Gas is not part of the linking refactor. Broader opcode expansion follows a verified gas baseline.

## 9. Verification requirements for future changes

| Requirement | Concrete acceptance evidence for a future change |
|---|---|
| Long forward references | `6002` repeated 46 and 47 times, with EOF and explicit STOP. All four complete normally with the expected number of twos. Exercise an actually taken distant exceptional jump with a seeded full stack as well as successful execution that skips it. |
| Correct external binding | Decode final JAL words independently. Check `rd=x0`, site alignment and bounds, signed displacement, and exact target publisher PC. Test multiple occurrences and both DUP1 destinations. If handwritten encoding survives, include `−4`, aligned range edges, and rejection outside the supported range. |
| Local wrapper integrity | Repeated ADD/SUB occurrences must call their own leaf and continue after it. Preserve operand ordering, carry/borrow, exact aliasing, `s0/s1`, and unchanged source leaf bytes where contractual. Exercise repeated POP/DUP1 labels too. |
| Artifact and entry-point parity | Compile the same bytes independently of initial stack. Run empty, one-word, valid seeded, depth 1023 and depth 1024 entries as appropriate. Compare guest status, exception code, `s1`, live stack and dynamic metrics. Check exact assembly/bytes only for an explicitly designated compatibility contract. |
| Publication and limits | Validate code bounds, publisher start PCs, status-last ordering, sentinel exclusion, and distinction between guest exception, malformed result, emulator fault, and instruction-limit error. Keep independent hand-authored ABI tests. |
| Gas boundaries after ADR approval | For every paid opcode test below, equal and above cost, including zero and large `u64` budgets. STOP at zero and empty/fallthrough preserve gas. OOG has no opcode effects. Stack errors settle to zero. Check both OOG and stack failure on full capacity. Validate `limit − remaining` and reject inconsistent publications. |
| Required Cancun vectors | `60020100`, budget 20 → guest underflow, settled remaining `0`. `5a5a00`, budget 10 → `[8,6]`, remaining `6`. `5a00`, budget 2 → `[0]`, remaining `0`; budgets 0 and 1 → OOG. Execute applicable cases through isolated and whole-program guest APIs. |
| Independent conformance and metrics | Preserve directed and seeded transaction tests. Add frame-equivalent comparisons for termination, settled gas and successful stacks. Preserve raw oracle output and Ferreth-specific exception invariants. Verify core hashes separately from expected increases in metered guest size and instruction counts. |

These are proposed regression requirements. The existing workspace run verifies the present tests. It does not implement the new long-range or gas regressions.

## 10. Deferred work and open decisions

- **Compatibility.** Does any maintained consumer require the exact historical PUSH1 display, exact machine bytes, the two-publisher layout, or just equality between sequence and bytecode APIs? The history establishes intent at introduction, but cannot answer for current consumers.
- **ADR-002 policy.** Confirm deterministic diagnostic precedence, stable exception codes, live versus published gas fields, and exact extension offsets. The log’s example of two appended `u64`s is a plausible layout. ADR-002 remains the proposal under review.
- **Artifact metrics.** Decide whether external users need a single static exceptional publisher length. Once publishers can differ, per-publisher information or the observed execution metrics is more precise.
- **Public API.** Decide whether compile-once/run-many should be part of the supported high-level library interface and how legacy convenience calls would receive a frame budget.
- **Deferred.** Opcode expansion beyond the proposed GAS milestone, dynamic gas, refunds, reversible state, nested frames, EVM control flow, shared arithmetic or gas subroutines, general relocation formats, and alternative execution backends.

Additionally defer canonical opcode identity, duplicate CLI summary decoding, whole-bytecode translation CLI and public artifact execution API improvements unless needed for the selected composition work. Track benchmark stdout contamination independently. The exact implementation of static termination accounting for different publisher lengths remains open. The maintainer has relaxed exact textual compatibility; any actual external consumer constraints still need an inventory.

## 11. Reproducibility appendix

I inspected the checkout with `git status --porcelain=v1 --branch --untracked-files=all`, `git rev-parse HEAD`, `git hash-object`, `git log`, `git show`, `rg`, and numbered source reads. I used `git archive HEAD` for the temporary build. The principal executable commands were:

```sh
cargo test --frozen --workspace -- --test-threads=1
cargo tree --frozen -p ferreth-compiler --depth 1

cargo run --frozen -q -p ferreth-compiler -- translate --op ADD
cargo run --frozen -q -p ferreth-compiler -- translate --op PUSH1 --value 2
cargo run --frozen -q -p ferreth-compiler -- execute --push1-sequence 2,3
cargo run --frozen -q -p ferreth-compiler -- execute --op ADD --a 2 --b 3
cargo run --frozen -q -p ferreth-compiler -- execute --op SUB --a 2 --b 3
cargo run --frozen -q -p ferreth-compiler -- execute --op PUSH1 --value 2
cargo run --frozen -q -p ferreth-compiler -- execute --op POP
cargo run --frozen -q -p ferreth-compiler -- execute --op DUP1
cargo run --frozen -q -p ferreth-compiler -- decode --bytecode 600260030100
cargo run --frozen -q -p ferreth-compiler -- decode --bytecode 00fe60

cargo run --frozen -q -p ferreth-conformance -- bench guest --format json
cargo run --frozen -q -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json
```

I ran `execute --bytecode` for `6002`, `600200`, `60026003`, `600260030100`, `0100`, `00`, `60`, `00fe`, `5a00`, `6101`, `50`, and `80`. The expected current backend rejections were observed for GAS and PUSH2. The long-input reproduction can be run against the built CLI with:

```python
import subprocess

for n in (46, 47):
    for ending in ("", "00"):
        subprocess.run([
            "target/debug/ferreth-compiler", "execute", "--bytecode",
            "6002" * n + ending,
        ], check=False)
```

I also ran the existing direct `revm-frame --code HEX --gas N` and EELS `run_frame.py --code HEX --gas N` interfaces for the reference vectors in sections 5 and 9, including `--tx-gas 100000` on the EELS settlement and long-PUSH checks. Temporary `rustc` probes against the frozen build read artifact bytes, compared isolated and compiled results, exercised seeded exceptions, and inspected assembler labels, relocations, signed limits and branch relaxation. The first observer build used an unsupported `u64.into()` conversion for the test seed and failed to compile. The corrected observer parsed U256 values and completed. No project source or test needed a change.

**Verification totals.** The workspace passed **135 tests, 0 failed, 0 ignored**. That includes the seeded RC/REVM test with 1,000 successful and 100 exceptional generated cases. The arithmetic benchmark passed **1,012/1,012 per operation**; ADD and SUB each measured 34 words and 34 executed instructions per case. Hashes matched `0xf1fff32ff47962f3` and `0x9f008f320eeb7a7b` respectively. The guest benchmark returned five isolated and ten program measurements. For both benchmark reports, I parsed the JSON suffix only after verifying that raw stdout failed JSON parsing as recorded in LF-031. No formatter, linter, gas implementation, or architectural prototype was run.

**Final checkout verification.** `git diff --exit-code HEAD --` and `git diff --cached --exit-code` passed. Branch, commit, porcelain status, and the pre-existing untracked document’s hash matched their initial values. The checkout received no source, test, documentation, dependency, configuration, branch, commit, or report-file change.

## 12. Closing record

Today's manual walk made the linker understandable; the independent review exposed its present limits and a simpler candidate approach. The next session begins with evidence, not a rewrite. The work and the research continue in bounded, reviewable increments.

Long live Ferreth.

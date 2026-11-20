# Phase 2.1 — Production RV64 final-image verification

**Scope.** Check the machine code returned by the three existing guest
linkers. The baseline is `a38326dfa1b9b5f85717d98c2b09a46387aada93`.
ADR-001 governs publication and the executable region. This phase applies
the transfer checks from RF-008–011 to the fixed production images.
[ADR-001](../../docs/adr/001-evm-guest-state-abi.md) ·
[Phase 1.3 evidence](phase-1/results/1.3-independent-verification.md) ·
[Phase 2.0 readiness](phase-2.0-readiness.md)

## RF-017 — Independent checks for the fixed guest images

The private function `rv64::verify::verify_final_image` takes the final byte
slice, a sequence of requested fragment kinds, the route's publisher layout,
and the ordered publisher PCs that the artifact will expose. The compiler
derives the fragment sequence from the lowered operations and explicit STOP
flag, or from the PUSH1 input count. The isolated linker declares one
arithmetic fragment. The check runs after assembly reassembly equality and
`validate_generated_code`, before any artifact is returned. A mismatch uses
the existing `CompileError::Link` or `OrchestratorError::Link` variant with
a verification diagnostic. The existing code-region failure still uses the
`Layout` variant.

| Route | Checkpoint | Publisher order |
| --- | --- | --- |
| PUSH1-only, including empty input | `compile_push1_program` | Normal, overflow |
| Mixed bytecode or explicit STOP | `compile_supported_program` | Normal, overflow, underflow |
| Isolated ADD and SUB | `link_guest_arithmetic` | Normal, underflow |

[Compiler checkpoints](../../crates/ferreth-compiler/src/compiler.rs) ·
[Isolated checkpoint](../../crates/ferreth-compiler/src/orchestrator.rs) ·
[Verifier](../../crates/ferreth-compiler/src/rv64/verify.rs)

Each private site specifies a zero-based word index within its fragment, a
transfer kind and its relevant fields, and an absolute target PC. The
fragment start and word index determine the site PC. A checked subtraction
of `ENTRY_PC` converts an absolute PC into an image byte offset. The target
is computed with checked signed addition to the site PC. Both PCs require
four-byte alignment and a complete word inside the loaded image. The
existing code-region check excludes the sentinel slot from the image.

The decoder reads little-endian words directly. JAL (`0x6f`) extracts `rd`
and reconstructs signed immediate bits `[20|10:1|11|19:12]`. B-type (`0x63`)
extracts `funct3`, `rs1`, and `rs2` and reconstructs signed bits
`[12|10:5|4:1|11]`. Both formats imply immediate bit 0 is zero. The decoded
PC must equal the independently expected PC. These checks use neither
assembler labels nor relocation records. [RISC-V unprivileged ISA](https://docs.riscv.org/reference/isa/unpriv/rv32.html)

### Source-derived completeness contract

All numbers in the transfer table are zero-based word indices. A
continuation is the first word after the fragment. All JALs use `x0`
except the call to the arithmetic core, which uses `ra` (`x1`).

| Fragment | Width in words | Required direct transfers |
| --- | ---: | --- |
| PUSH1 | 11 | `bgeu s1,t0` at 2 → 10; 9 → continuation; 10 → overflow publisher |
| POP | 4 | `bgeu s0,s1` at 0 → 3; 2 → continuation; 3 → underflow publisher |
| DUP1 | 16 | `bgeu s0,s1` at 0 → 14; `bgeu s1,t0` at 3 → 15; 13 → continuation; 14 → underflow; 15 → overflow |
| ADD or SUB | 43 | `bltu s1,t0` at 1 → 8; `jal ra` at 5 → core word 9; 7 → continuation; 8 → underflow publisher |
| STOP | 1 | 0 → immediately following normal publisher |

The verifier checks the arithmetic call's `PC+4` stack-update word and the
last word of the 34-word leaf (`ret`) against literal encodings. The
publisher sections must start at the PCs implied by those fragment widths
and the publisher order. The normal publisher occupies seven words and
each exception publisher occupies eight. Their fixed words identify
the specific publication role, including `final_s1`, exception code,
status-last stores, and the existing `jalr zero, -4(t1)` sentinel transfer.
Checking the complete publisher bodies distinguishes overflow from underflow
even if the positions and all references are unchanged.

Each fragment kind supplies its complete, ordered list of required sites.
The scan checks every word in the fragment and rejects extra JAL, B-type,
or JALR opcodes, non-32-bit instruction widths, and duplicate, unordered,
or out-of-fragment site indices. It advances through every declared
fragment and publisher and requires the cursor to end exactly at the end
of the image. This catches an omitted fragment or inserted local transfer
when its words disagree with the supplied fixed layout. The caller must
derive the requested fragment sequence and select the correct route.

The decoder covers JAL and B-type only. It compares the existing indirect
return and sentinel instructions as literal words. It does not infer
runtime register values, validate general RV64 instructions, or prove that
other instructions preserve guest state. Arithmetic-core contents and
opcode bodies remain subject to runtime and differential tests. The
contract is specific to contiguous, instruction-only images with the
current generator widths, and rejects branch relaxation inside those
fragments. A future layout change needs a corresponding reviewed contract.
[Generators](../../crates/ferreth-compiler/src/rv64/opcodes) ·
[Executable bounds](../../crates/ferreth-compiler/src/rv64/memory.rs)

## Regression evidence

All 13 verifier unit tests and both public integration tests passed.

| Coverage | Observed result |
| --- | --- |
| Independent literal JAL and B-type fixtures | Forward and backward displacements and both signed encoding limits for each format decoded as expected. Valid aligned backward targets were accepted. |
| LF-027 sign-bit mutation | `0xffdff06f` at `0x80000004` reached `0x80000000` with displacement −4. Clearing instruction bit 31 produced `0x7fdff06f` with displacement +1,048,572 and was rejected at out-of-image target `0x80100000`. |
| Fields, destinations, and bounds | Rejected wrong opcodes, JAL `rd`, B-type condition and both source registers, aligned but unintended targets, misaligned sites, truncated site and target words, targets before or after the image, and positive and negative PC wraparound. The last complete target word was accepted. An assembler-accepted `jal zero, 6` was rejected for four-byte target alignment. |
| Completeness and expansion | Rejected missing, duplicate, unordered, and out-of-fragment site declarations, incorrect fragment and publisher inventories, undeclared JALR, non-32-bit widths, and trailing image words. Pinned `asm-rs` emitted the tested `BNE +8` followed by `JAL +5000` for a distant BEQ. Declaring the inverted first leg by itself still exposed the inserted JAL. |
| Reassembled wrong intent | Assembly and machine bytes that agree with each other were rejected for a wrong PUSH1 continuation, either misrouted DUP1 publisher, a wrong ADD core PC, a wrong SUB call register, a changed return or `PC+4` anchor, and a STOP jump to overflow. Swapped exception bodies at unchanged PCs and reordered normal publication words were rejected. |
| Public production boundaries | Successful PUSH1 bytecode and sequence artifacts matched. The mixed case exercised every supported fragment kind. Isolated ADD and SUB retained their two publishers and completed both the seeded normal path and empty-stack underflow. |

The existing memory tests for the exact end at `EXIT_PC` and for overlap by
one word also passed. The full workspace run includes the existing seeded
REVM Cancun differential test and both 46-PUSH1 controls. The Phase 1
research suite still verifies the two-leg relaxed-branch behavior separately.
[Verifier tests](../../crates/ferreth-compiler/src/rv64/verify/tests.rs) ·
[Integration tests](../../crates/ferreth-compiler/tests/rv64_final_image.rs) ·
[Phase 1.3 fixtures](phase-1/tests/independent_verification.rs)

### Frozen verification

Ran with Rust and Cargo 1.98.1 on `aarch64-apple-darwin`, `asm-rs 0.2.0`,
`rvemu 0.0.11`, `alloy-primitives 1.7.3`, and REVM 43.0.3. The baseline
and candidate used separate source trees and Cargo target directories.
Their workspace manifests and lockfiles matched the repository byte for
byte. Lockfile Git blob `a8dca4b2d37c0c59561c4ddb8c666c272bc5a98b`.

| Run | Discovered | Passed | Failed | Ignored | Exit |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline workspace | 164 | 159 | 4 | 1 | 101 |
| Phase 2.1 workspace | 179 | 174 | 4 | 1 | 101 |
| Phase 2.1 compiler | 141 | 137 | 4 | 0 | 101 |
| Phase 1 research | 23 | 22 | 0 | 1 | 0 |
| Capacity probe, explicit ignored-only rerun | 1 | 1 | 0 | 0 | 0 |

Workspace discovery exited 0 in both snapshots. Its inventory matched
execution by target and test name. Every baseline case retained its status
and all 15 additions passed. Both full runs had zero measured or filtered
tests. The probe remained ignored in ordinary runs and reproduced the
100,000-label limit and Phase 2.0 image-boundary observations on its
dedicated rerun. Workspace formatting and strict frozen Clippy passed in
both snapshots.

| Unchanged LF-026 regression | Rejected JAL displacement |
| --- | ---: |
| `repeated_push1_47_eof_completes_normally` | +2056 |
| `repeated_push1_47_eof_full_stack_publishes_overflow` | +2056 |
| `repeated_push1_47_stop_completes_normally` | +2060 |
| `repeated_push1_47_stop_full_stack_publishes_overflow` | +2060 |

Both snapshots reported identical `ImmediateOverflow` diagnostics,
including limits −2048 through +2047 and line 12, column 5. All four
fail before final-image verification, during display-assembly reassembly.
Their test source remains at blob `821e0968cf47fa0c83edc05e2cbf0746e484de8e`.
The negative fixture detects LF-027 sign corruption. The two legacy JAL
encoders still contain the latent defect. [Phase 0 reproduction](README.md)

## Before and after artifact and runtime parity

The same Phase 0.3 observer ran against separately built pre-change and
Phase 2.1 binaries. All 236 files from the 41 specimens and the sequence
equality flag matched by name and bytes. They include 41 machine images,
41 assembly texts, 41 label and publisher maps, 30 public compiler metadata
records, 41 ABI publication traces, and 41 execution records. The observer
asserted exact reassembly and matched compiler publisher metadata against
assembly labels where the API exposes it. For isolated ADD and SUB it
derived the ordered publisher PCs from the returned assembly, checked
reassembly, and captured the publisher actually reached during replay.
[Observer source](phase-0.3/observe.rs)

A second replay used a temporary observer copy with a 32-byte `0xa5` guard
at `EVM_STACK_REGION.end()`. Every guest preserved that guard and native
`sp`. Live `s0` matched the ABI stack base and live `s1` matched `final_s1`.
The 277 files, including 41 additional register and guard records, matched
byte for byte. Both captures compare full live stacks and the publisher
trace before execution and after each publisher instruction. Samples with
identical field values cannot identify a store of the existing value by
themselves. The captured assembly and the verifier's publisher words
provide that instruction-level check.

Representative rows list absolute publisher PCs and observed RV64 counts.

| Guest | Bytes | Selected publisher PC | Status / code | Final `s1` | Body / publisher / total |
| --- | ---: | --- | --- | --- | --- |
| Empty bytecode, EOF | 60 | `0x80000000` | Normal / 0 | `0x80101000` | 0 / 7 / 7 |
| PUSH1 `6002`, empty stack | 104 | `0x8000002c` | Normal / 0 | `0x80101020` | 10 / 7 / 17 |
| PUSH1 `6002`, full stack | 104 | `0x80000048` | Exception / 1 | `0x80109000` | 4 / 8 / 12 |
| `600260030100` | 356 | `0x80000108` | Normal / 0 | `0x80101020` | 63 / 7 / 70 |
| `60058050` | 216 | `0x8000007c` | Normal / 0 | `0x80101020` | 27 / 7 / 34 |
| DUP1, full stack | 156 | `0x8000005c` | Exception / 1 | `0x80109000` | 5 / 8 / 13 |
| DUP1, empty stack | 156 | `0x8000007c` | Exception / 2 | `0x80101000` | 2 / 8 / 10 |
| Isolated ADD and SUB, stack `[2, 3]` | 232 | `0x800000ac` | Normal / 0 | `0x80101020` | 42 / 7 / 49 |
| Isolated ADD and SUB, empty stack | 232 | `0x800000c8` | Exception / 2 | `0x80101000` | 3 / 8 / 11 |

The guest benchmark matched across all five opcode and ten program cases.
The standalone arithmetic benchmark matched for all 1,012 cases per
operation. Both leaves used 34 static and 34 executed words per case.
ADD and SUB FNV-1a hashes were `0xf1fff32ff47962f3` and
`0x9f008f320eeb7a7b`. The two raw benchmark stdout streams matched,
including the existing rvemu warnings because `dtc` was unavailable.
The JSON payloads started at byte 326 for the core benchmark and 2445
for the guest benchmark. Both stderr streams were empty. These captures
exercise one executor and host and measure RV64 instructions.

| Matched evidence | SHA-256 in both snapshots |
| --- | --- |
| 236-file observer inventory | `1b6f8769058be4280ed66125a0a41d21721b6ab9490e6d32977cbc7b8707d6c5` |
| 277-file guard observer inventory | `48ef920f060203d928aa1c581c5e9a35217de3cb140eb20635c43f3fadff75b8` |
| Core benchmark stdout | `f4f44b247a0d506779e306b2956d147ba4a216d67a8293295f641660a64c9f71` |
| Guest benchmark stdout | `5fe319d3b352fd0e800decf469b8b17553df9c3848359a9f77f0d363a1c123af` |

Each inventory digest hashes the concatenation, sorted by specimen
filename, of one line per file in the form `<sha256>  <filename>\n`.

## Reproduction

Create two source snapshots with `git archive` of
`a38326dfa1b9b5f85717d98c2b09a46387aada93`. Overlay the six Rust files in
the change list below on the candidate. Run from within each snapshot
with a fresh, distinct `CARGO_TARGET_DIR` outside the source trees. The
isolated working directories also contain rvemu's DTS side effects.

```sh
cargo fmt --all -- --check
cargo clippy --frozen --workspace --all-targets -- -D warnings
cargo test --frozen --workspace -- --list
cargo test --frozen --workspace --no-fail-fast -- --test-threads=1
```

Run these additional gates on the candidate.

```sh
cargo test --frozen -p ferreth-compiler --lib rv64::verify::tests -- --test-threads=1
cargo test --frozen -p ferreth-compiler --test rv64_final_image -- --test-threads=1
cargo test --frozen -p ferreth-compiler --no-fail-fast -- --test-threads=1
cargo test --frozen -p ferreth-linking-spike -- --test-threads=1
cargo test --frozen -p ferreth-linking-spike --test capacity_probe -- --ignored --nocapture --test-threads=1
```

For each snapshot, set `run` to a fresh absolute capture directory and
execute the following from the snapshot root. Comparing Cargo build
records selects the matching library variants when a target directory
contains both compiler-only and workspace dependency feature sets.

```sh
mkdir -p "$run"
cargo build --frozen -p ferreth-conformance --bin ferreth-conformance \
  --message-format=json > "$run/build.jsonl"
python3 - "$run" "$CARGO_TARGET_DIR" "$PWD/research/guest-linking/phase-0.3/observe.rs" <<'PY'
import json
from pathlib import Path
import subprocess
import sys

run, target, source = map(Path, sys.argv[1:])
events = [json.loads(line) for line in (run / 'build.jsonl').read_text().splitlines()]
args = ['rustc', '--edition=2024', '-L', f'dependency={target}/debug/deps']
for crate in ('ferreth_compiler', 'alloy_primitives', 'asm_rs'):
    rlib, = [file for event in events if event['reason'] == 'compiler-artifact'
             and event['target']['name'] == crate for file in event['filenames']
             if file.endswith('.rlib')]
    args.extend(['--extern', f'{crate}={rlib}'])
subprocess.run([*args, source, '-o', 'observe'], cwd=run, check=True)
PY
(
  cd "$run"
  ./observe specimens > observer.stdout 2> observer.stderr
  "$CARGO_TARGET_DIR/debug/ferreth-conformance" bench --op ALL --cases 1000 --seed 42 --format json > core.stdout 2> core.stderr
  "$CARGO_TARGET_DIR/debug/ferreth-conformance" bench guest --format json > guest.stdout 2> guest.stderr
)
```

For the guard run, compile a temporary copy of the observer with this
initialization immediately before `initial_sp` is sampled.

```rust
let guard_address = EVM_STACK_REGION.end();
let guard = [0xa5; 32];
executor.write_memory(guard_address, &guard).unwrap();
```

Add the following after `assert_eq!(initial_sp, final_sp)` and capture to a
separate `guarded` directory.

```rust
let live_s0 = executor.read_register(IntegerRegister::EVM_STACK_BASE);
let live_s1 = executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE);
let mut live_guard = [0; 32];
executor.read_memory(guard_address, &mut live_guard).unwrap();
assert_eq!(live_s0, base);
assert_eq!(live_s1, final_s1);
assert_eq!(live_guard, guard);
fs::write(
    out.join(format!("{id}.registers.tsv")),
    format!("{live_s0:#x}\t{live_s1:#x}\t{final_sp:#x}\t{live_guard:02x?}\n"),
).unwrap();
```

Compare each file and stream byte for byte. The local run retains raw
streams, logs, and specimens in `baseline-run/` and `candidate-run/`
under `/private/tmp/ferreth-phase21.AD5Px6De/`. An initial targeted test
build required explicit reference types in a test helper. An initial
candidate observer link attempt found multiple `.rlib` variants. Both
recoveries are reflected in the successful gates and recipe above.

## Diff and exit gate

| Files | Change |
| --- | --- |
| `crates/ferreth-compiler/src/compiler.rs` | Invoke the check at the PUSH1-only and mixed artifact boundaries. |
| `crates/ferreth-compiler/src/orchestrator.rs` | Invoke the check at the isolated ADD/SUB boundary. |
| `crates/ferreth-compiler/src/rv64/mod.rs` | Register the crate-private module. |
| `crates/ferreth-compiler/src/rv64/verify.rs` | Fixed-layout site and publisher checks and independent decoder. |
| `crates/ferreth-compiler/src/rv64/verify/tests.rs` | Literal, assembler, and final-image mutation tests. |
| `crates/ferreth-compiler/tests/rv64_final_image.rs` | Public compile and execution integration tests. |
| `research/guest-linking/README.md`, this report | Index and production verification evidence. |

The scoped diff adds read-only verification and maps failures to existing
link errors. The emitters, assembler configuration, patching algorithms,
executor, public APIs, and published ABI are unchanged. The pre-existing
architecture document edit retained blob `141d4a12172e3152716a5a92e869e05fda95c918`.

**Phase 2.1 exit gate met.** All new acceptance and rejection tests passed,
artifact and runtime parity held for the captured guests, and the workspace
retained only the four expected LF-026 failures. The next action is
maintainer review of Phase 2.1. Phase 2.2 awaits authorization.

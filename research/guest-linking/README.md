# RV64 guest linking Phase 0 evidence

Start with [How Ferreth links an RV64 guest](../../docs/research/ferreth-guest-linking.md) for the canonical account of the current architecture and pending Phase 3 work. The [original linking investigation](ferreth-linking-architecture.md) preserves the earlier design alternatives, LF findings and historical roadmap. The phase summaries below describe their original checkpoints. For current compiler and benchmark commands, see the [CLI runbook](../../docs/runbooks/cli.md) and [benchmark runbook](../../docs/runbooks/benchmark.md).

Phase 0 established a reproducible artifact and execution baseline, isolated
the long forward JAL failure (LF-026), and reproduced the signed JAL encoder
defect (LF-027). The test checkpoint exercises desired guest behavior under
the accepted ABI. The linker and the proposed gas architecture require
separate review.
[ADR-001](../../docs/adr/001-evm-guest-state-abi.md) ·
[ADR-002](../../docs/adr/002-cancun-evm-gas-metering.md)

The capture used Cargo/rustc 1.98.1 on aarch64-apple-darwin, asm-rs 0.2.0,
rvemu 0.0.11, REVM 43.0.3 and revm-interpreter 43.0.2. Use Bash, Python 3
and locally available dependencies from the frozen lockfile. The baseline
revision is `2a49a0a3a157a1e505ab46d2dc66f333606a4802`. The regression
checkpoint is `2aa061cf46794588b8d95d3b252dda2e45c1f338`.

## Phase 0.3 artifact and benchmark capture

From the repository root, create a separate checkout and use a fresh output
directory. The script reads the frozen dependency trees and runs the arithmetic
and guest benchmarks before compiling the standalone Rust observer. It records
raw streams and exits, and writes `baseline.json` and the specimen files.

```sh
repo="$PWD"
out="$repo/runs/phase-0/local"
mkdir -p "$out"
git clone --shared --no-checkout "$repo" "$out/baseline-source"
git -C "$out/baseline-source" switch --detach 2a49a0a3a157a1e505ab46d2dc66f333606a4802
bash research/guest-linking/phase-0.3/reproduce.sh "$out/baseline-source" "$out/baseline-capture"
```

The observer captures ordered machine words and assembly, publisher PCs,
complete stacks, final `s1`, and static and dynamic RV64 instruction counts
for 41 cases. It checks eight entry-point comparisons and replays publisher
metadata instruction by instruction. Identical consecutive metadata samples
cannot distinguish a write of the same value from no write. Use the assembly
to identify those stores.

The earlier capture and the audit replay produced identical specimen files
and benchmark and observer streams. The reports record their actual branch
status and absolute dependency-tree source paths. The original report's SHA-256
and per-artifact SHA-256 values are in the compact baseline. Arithmetic ADD and
SUB each passed 1,012 cases at 34 static words and 34 dynamic instructions per
case. The FNV-1a hashes are `0xf1fff32ff47962f3` and `0x9f008f320eeb7a7b`.
The guest benchmark recorded five opcode and ten program measurements.
[Artifact baseline](../../runs/phase-0/artifact-baseline.json) ·
[Arithmetic benchmark](../../runs/phase-0/arithmetic-baseline.json) ·
[Guest benchmark](../../runs/phase-0/guest-benchmarks.json)

With `dtc` absent, rvemu printed warnings to stdout. The recorded JSON
payloads began at bytes 326 and 2445 for the arithmetic and guest reports.
The collector verifies the warning prefix and parses the remaining bytes
through EOF. The raw streams remain available in the local captures.
Instruction counts describe RV64 execution. The Cancun runner's remaining gas
describes the EVM frame. [LF-031 evidence](../lf-031/README.md) · [Current stdout requirements](../../docs/runbooks/benchmark.md#json-stdout)

## Phases 0.4 and 0.5 boundary and regression capture

Use the detached regression checkpoint `2aa061cf46794588b8d95d3b252dda2e45c1f338`
to reproduce the historical LF-026 failures. The current symbolic path repairs
LF-026 and the historical probe depends on legacy functions and APIs from the
checkpoint. The probe runs Ferreth, direct REVM Cancun, and EELS Cancun with
Python 3.14.5 and execution-specs commit
`d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3`.

Run the following from the current repository root. The commands default to
its pinned EELS environment and pass it to the historical probe. Set
`EELS_PYTHON` to an absolute interpreter path to use an equivalent environment.
[EELS setup](../eels-cancun/README.md)

```sh
repo="$PWD"
replay=$(mktemp -d)
evidence="$repo/research/guest-linking/phase-0.4-0.5"
export EELS_PYTHON="${EELS_PYTHON:-$repo/research/eels-cancun/.venv/bin/python}"
git clone --shared --no-checkout "$repo" "$replay/source"
git -C "$replay/source" switch --detach 2aa061cf46794588b8d95d3b252dda2e45c1f338
bash "$evidence/reproduce-probes.sh" "$replay/source" "$replay/capture"
bash "$evidence/reproduce-tests.sh" "$replay/source" "$replay/capture/tests"
python3 "$evidence/verify.py" "$replay/capture" "$replay/capture/tests"
```

Both 46-PUSH inputs complete normally. Both 47-PUSH lengths are within the
validated code layout. Their projected publisher positions come from the
46-PUSH artifacts plus one 44-byte fragment. The linker renders a numeric
JAL operand and fails when asm-rs reassembles it at +2056 bytes (EOF) or
+2060 (STOP). The isolated symbolic JAL experiment accepts those distances.
The four RED tests fail before any 47-PUSH guest executes. The two full-stack
tests require a published overflow, exact preservation of all 1,024 seeded
words and final `s1=0x80109000`. The public artifact check decodes the first
PUSH's JAL and compares its target with the overflow publisher PC.
[Regression tests](../../crates/ferreth-compiler/tests/rv64_push1_program.rs) ·
[Linking probe output](../../runs/phase-0/linking-probe.txt)

The probe extracts the private encoders for independent characterization.
For a requested displacement of −4, both emit `0x7fdff06f` (+1048572).
Symbolic assembly emits `0xffdff06f` (−4). Source inspection found that
current publisher patches point forward. The permanent regressions use public
APIs and independently decode emitted JAL words.

Both Cancun references complete each 47-PUSH input with 47 twos and zero
remaining frame gas from 141. EELS reports terminal PCs 94 and 95 for EOF
and STOP. Direct REVM reports 95 for both. The audit replay matched the prior
probe and reference stdout byte for byte.
[EELS EOF](../../runs/phase-0/eels-eof.txt) ·
[EELS STOP](../../runs/phase-0/eels-stop.txt) ·
[REVM EOF](../../runs/phase-0/revm-eof.txt) ·
[REVM STOP](../../runs/phase-0/revm-stop.txt)

The audit captured a fresh baseline run, discovery, fail-fast and complete
workspace results. The tracked inventory records Cargo exit statuses,
per-target summaries, counts cross-checked from individual outcomes, and the
four rejected displacements. Raw streams and full specimen captures are in
the local `runs/phase-0/local/` directory on the audit machine.
Use distinct `CARGO_TARGET_DIR` values for the baseline and integrated
checkouts. A shared target directory produced a stale 17-test binary in one
audit run. Baseline discovery should contain 135 tests and RED discovery
should contain 141.
[Workspace inventory](../../runs/phase-0/workspace-verification.json) ·
[Engineering log](../../docs/bitacora/2026-11-20.md)

## Production migration readiness

Phase 1.7 recommends symbolic guest composition, subject to maintainer review.
The [Phase 2.0 readiness report](phase-2.0-readiness.md) inventories current
linking and publisher ownership, records experimentally observed assembler and
Ferreth capacity limits, identifies compatibility decisions, and defines
bounded implementation and rollback gates. It does not authorize or implement
production migration.

## Production final-image verification

The [Phase 2.1 report](phase-2.1-final-image-verification.md) records the
private verifier at all three production linker checkpoints, its fixed-layout
contract, and the before/after artifact and execution comparison. The
Phase 2.1 gate passed with the four expected LF-026 failures still active.

## Private PUSH1 and STOP composer

The [Phase 2.2 report](phase-2.2-symbolic-guest-composer.md) records the
private composer, shared publisher source, parity and distant overflow
regressions, and measured executable bounds. The public compiler routes
still exercise the four LF-026 controls. Phase 2.2 ends at maintainer review.

## Opcode-independent fragment contract

The [Phase 2.3a report](phase-2.3a-fragment-contract.md) records the generated
fragment/reference contract, declaration-derived publisher inventory, final
image verification and retained PUSH1 fixed-layout gate. Only PUSH1 and STOP
are symbolically generated; public compilation remains on the legacy routes.
Phase 2.3a stops for maintainer review before mixed-opcode composition.

## Symbolic POP and DUP1

The [Phase 2.3b report](phase-2.3b-pop-dup1.md) records private symbolic POP
and DUP1 fragments, declaration-derived three-role publisher inventory,
fixed-layout and runtime verification, label-density probes, and workspace
counts. Public compilation remains unchanged and the milestone stops for
maintainer review.

## POP/DUP1 boundary hardening

The [Phase 2.3b.1 report](phase-2.3b.1-boundary-hardening.md) traces the large
DUP1 errors to JAL relocations outside the executable domain, measures the
POP label and image ceilings, and records the directed ADR-001 runtime parity
and verifier regressions. The private POP path accepts 49,999 EOF fragments
and rejects 50,000 for label count even though the layout fits. The
maintainer closed RF-019 and accepted RF-020 as a known implementation
capacity limit. Phase 2.3b.1 is reviewed and accepted.

## Symbolic ADD and SUB

The [Phase 2.3c report](phase-2.3c-symbolic-arithmetic.md) records RF-022,
private ADD/SUB fragments, independent local call and return checks,
exact core identity, and runtime parity with the public legacy route.
The implementation followed the [Phase 2.3c plan](phase-2.3c-plan.md)
and now awaits maintainer review.

## Private integration and public cutover review

The [Phase 2.3d audit](phase-2.3d-integration-review.md) records the six-opcode
matrix, RF-023 through RF-025, frozen verification and isolated arithmetic
image identity. The [Phase 2.4 plan](phase-2.4-cutover-plan.md) identifies
the routing and public compatibility decisions. The
[Phase 2.4 public cutover review](phase-2.4-public-cutover.md) records the
RF-026 regression repair, the RF-027 resident-word policy approval and
verification for maintainer review.

## Final cleanup

The [Phase 2.5 cleanup report](phase-2.5-final-cleanup.md) records the legacy
linker inventory, the narrowed dead-code allowances, the test-oracle
disposition and the full validation gate. The
[completed Human-Led Walk](ferreth-human-led-walk.md) records the checkpoint
evidence. Phase 3.3 validation and the release decision remain separate.

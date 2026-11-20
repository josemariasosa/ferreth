# RV64 guest linking Phase 0 evidence

Phase 0 established a reproducible artifact and execution baseline, isolated
the long forward JAL failure (LF-026), and reproduced the signed JAL encoder
defect (LF-027). The test checkpoint exercises desired guest behavior under
the accepted ABI. The linker and the proposed gas architecture require
separate review. [Linking research](../../docs/research/ferreth-linking-architecture.md) ·
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
describes the EVM frame. [LF-031](../../docs/research/ferreth-linking-architecture.md#additional-findings-from-independent-review)

## Phases 0.4 and 0.5 boundary and regression capture

Use the regression checkpoint or a checkout containing it. The probe runs
Ferreth, the frozen direct REVM Cancun runner, and the EELS Cancun runner
with Python 3.14.5 and execution-specs commit
`d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3`. The EELS interpreter defaults
to `research/eels-cancun/.venv/bin/python`. Set `EELS_PYTHON` to use an
equivalent pinned environment. [EELS setup](../eels-cancun/README.md)

```sh
repo="$PWD"
out="$repo/runs/phase-0/local/boundary-replay"
evidence="$repo/research/guest-linking/phase-0.4-0.5"
bash "$evidence/reproduce-probes.sh" "$repo" "$out"
bash "$evidence/reproduce-tests.sh" "$repo" "$out/tests"
python3 "$evidence/verify.py" "$out" "$out/tests"
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

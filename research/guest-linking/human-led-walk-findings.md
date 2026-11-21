# Ferreth — Human-Led Walk findings

The [walkthrough](ferreth-human-led-walk.md#session-handoff--2026-11-21) holds the session evidence ledger. [H-WALK-002](ferreth-human-led-walk.md#h-walk-002--symbol-naming-and-namespace-ownership) and [H-WALK-003](ferreth-human-led-walk.md#h-walk-003--independent-continuation-verification) remain in their original review context.

## H-WALK-001 — Decoder mixes decoding and reachability

**Classification** — Accepted limitation for v0.1 and architectural debt before `JUMP`/`JUMPI`.

**Source evidence (`579e4f5`)** — `decode_detailed()` emits the first `STOP` and ends the scan. It returns `DecodeError::UnsupportedOpcode` for bytes outside its recognized set before `STOP`, including valid EVM opcodes such as `MUL` (`0x02`). It accepts `PUSH1` through `PUSH32`, and the symbolic frontend separately rejects `PUSH2` through `PUSH32` with `CompileError::UnsupportedInstruction`. [Decoder](../../crates/ferreth-compiler/src/lib.rs#L106) · [Disassembler opcode table](../../crates/ferreth-compiler/src/disassembler.rs#L175) · [Symbolic frontend](../../crates/ferreth-compiler/src/compiler/symbolic.rs#L96)

Existing decoder tests specify that `60 05 00 02` yields `PUSH1 5; STOP` and that `61 02 01 02` reports unsupported opcode `0x02` at byte offset 3. These expectations were checked by source inspection. [Decoder tests](../../crates/ferreth-compiler/src/lib.rs#L318)

The first `STOP` ends the only execution path in the current six-opcode subset. With dynamic control flow, an earlier jump can reach instructions beyond that `STOP`.

**Desired responsibilities**

- The decoder identifies EVM instructions and immediates across the byte stream, preserving original byte offsets independently of compiler opcode coverage.
- The frontend determines which instructions are relevant and supported for compilation.
- The composer links generated RV64 fragments through their declared interfaces without opcode-specific knowledge.

**Decision and status** — Documented and deferred during the Human-Led Walk. No correctness defect was demonstrated in the supported subset. Review the separation before introducing `JUMP` or `JUMPI`.

**Future regression requirements** — For `60 05 00 02`, full-stream decoding should identify `MUL` at offset 3 while compilation of the reachable prefix still completes normally with `[5]` from an empty stack. With `JUMP` support, `60 04 56 00 5b 00` should reach `JUMPDEST` at offset 4 from an empty stack and complete normally with an empty stack. Verification of these future behaviors is pending.

## H-WALK-004 — Reusable conformance scenarios

**Classification** — Phase 3 design candidate. The maintainer's decision and ownership are pending.

**Evidence** — The reviewed code has a standalone EELS Cancun frame CLI, a Cancun REVM transaction differential harness with directed cases and fixed-seed stack-aware generation, and separate direct-frame evidence and benchmark specimens. The review found no demonstrated shared scenario catalog across those routes. The current generator is scoped to the supported opcode subset. [November 21 conformance evidence](ferreth-human-led-walk.md#eg-evidence-from-the-november-21-continuation) · [Conformance roadmap](../../docs/research/ferreth-evm-conformance.md#8-next-steps--incremental-conformance-roadmap)

**Candidate** — Evaluate named scenarios with bytecode, fork, optional initial stack, expected observations and provenance. Consider persistent EELS-derived fixtures alongside the existing REVM differential checks. Preserve deterministic regression seeds. Evaluate configurable seed exploration and fuzz testing separately. Keep reference execution conditions and raw outputs with each applicable observation so an isolated frame result can be distinguished from a transaction result.

**Acceptance examples for design review** — With an empty initial stack under Cancun, `600560030100` should complete successfully with `[8]`. Empty-stack `80` should exceptionally halt while preserving diagnostic provenance for EELS `StackUnderflowError`, Ferreth `StackUnderflow` and REVM `StackOverflow`. Keep the dedicated diagnostic regression and document its scope. A scenario consumer should distinguish a semantic mismatch from a diagnostic mismatch, and record any explicitly accepted discrepancy with its evidence. Include frame gas expectations only where execution conditions and supported observations permit comparison. These vectors have existing evidence as recorded in the ledger. A shared representation and its regression coverage remain proposals.

## H-WALK-005 — Guest benchmark warnings on JSON stdout

**Classification** — CLI output hygiene. The observation concerns machine-readable stdout. The benchmark payload matched the tracked values after removing the warnings. Resolution and ownership are pending.

**November 21 observation** — `cargo run -q -p ferreth-conformance -- bench guest --format json` captured rvemu device-tree warnings together with the JSON report. The reviewer manually removed those warning lines from `/tmp/ferreth-guest-bench.json` before running this comparison from the repository root:

```bash
diff -u \
  <(jq -S . runs/phase-0/guest-benchmarks.json) \
  <(jq -S . /tmp/ferreth-guest-bench.json)
```

The reported diff was empty. Warning removal changed the captured stream from warnings plus report to the JSON payload alone. `jq -S` sorted object keys in both inputs. No metric values were changed. The supplied session record contains no raw stdout capture or warning count. The Phase 2.4 ledger records the warning forms and raw stream hash for its own run. [Guest baseline](../../runs/phase-0/guest-benchmarks.json) · [Historical comparison and warning prefix](phase-2.4-public-cutover.md#benchmark-and-arithmetic-core-evidence) · [Benchmark runbook](../../docs/runbooks/benchmark.md)

**Source corroboration** — In the pinned rvemu 0.0.11 dependency, `Rom::new()` uses `println!` for the device-tree binary read error and the suggestion to install `dtc`, then continues with an empty device-tree blob. The guest benchmark CLI prints the completed report to stdout. This explains how an emulator diagnostic can precede the report even when the command succeeds. [Dependency pin](../../Cargo.lock) · [CLI output path](../../crates/ferreth-conformance/src/main.rs)

**Proposed regression if a fix is approved** — In an environment that triggers the device-tree warning path, capture stdout and stderr separately. Require successful exit, stdout that parses directly as one JSON object, and full payload equality with the tracked baseline. Preserve the diagnostics separately and retain the raw streams for review. No fix or regression was implemented in this walk.

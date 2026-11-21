# Ferreth — Human-Led Walk findings

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

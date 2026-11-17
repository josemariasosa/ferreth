# EVM ADD to RV64IM

This experiment identifies EVM ADD as opcode `0x01` and emits a complete GNU
assembly function implementing addition modulo 2^256. It does not implement
the EVM stack, gas, bytecode parsing, or complete observable parity with revm.

```sh
cargo run --manifest-path translator/Cargo.toml -- translate --op ADD
cargo test --manifest-path translator/Cargo.toml
```

## Calling convention

`evm_add` uses the RISC-V integer calling convention: `a0` points to the first
operand, `a1` to the second, and `a2` to the destination. Each points to four
8-byte-aligned `u64` limbs in little-endian limb order (least significant
first), using the target's native byte order. The destination may be exactly
the same address as either input, but must not otherwise overlap either
32-byte input region. The inputs may overlap each other. The function writes
four limbs at `a2`; it has no register return value.

The function reads `a0`-`a2`, clobbers caller-saved `t0`-`t5`, and preserves
all other registers. It uses no stack. Each limb adds the two inputs, detects
unsigned overflow with `sltu`, adds the incoming carry, and detects that
addition's overflow with a second `sltu`. The OR of the two flags is the carry
to the next limb. The carry out of limb 3 is discarded. `li` and `ret` are GNU
assembler pseudoinstructions; the resulting instructions are RV64I and thus
also valid on RV64IM.

For `a = [5, 0, 0, 0]` and `b = [7, 0, 0, 0]`, the written limbs are
`[12, 0, 0, 0]` (hex: `000000000000000c 0000000000000000
0000000000000000 0000000000000000`).

## End-to-end differential test

The integration test assembles the translator's generated source for RV64 with
the documented `asm-rs` Rust assembler and executes its machine code
instruction-by-instruction in the `rvemu` Rust emulator. It removes only the
GNU metadata directives, which do not encode instructions. The test initializes
the emulator's memory and `a0`/`a1`/`a2` registers directly, uses an `EBREAK`
return address for deterministic termination, and enforces a 256-instruction
limit. The generated function uses RV64I instructions, which are a subset of
RV64IM. It does not use a software fallback for RV64 execution.

The other path constructs EVM bytecode with `PUSH32`, `ADD`, `MSTORE`, and
`RETURN` and executes it through revm using the fixed Cancun hardfork. Each path
produces all 32 result bytes independently; the test compares those full
256-bit results for seven fixed cases and 1,000 deterministic randomized
inputs (seed `0x123456789abcdef0`).

Run from the repository root:

```sh
cargo test --manifest-path translator/Cargo.toml
cargo test --manifest-path translator/Cargo.toml --test rv64_execution -- --nocapture
```

The second command also prints an executed `ADD(5, 7)` example and its results.
The experiment checks only the arithmetic result modulo 2^256. It does not
verify EVM stack semantics, complete transaction behavior, revert handling,
other opcodes, or gas parity; the EVM bytecode consumes gas, but the RV64
function does not implement gas metering. Passing tests are experimental
evidence, not a formal correctness proof or universal EVM parity.

# EVM ADD and SUB to RV64IM

This experiment identifies EVM ADD (`0x01`) and SUB (`0x03`) and emits complete
GNU assembly functions implementing arithmetic modulo 2^256. It does not
implement the EVM stack, gas, bytecode parsing, or complete observable parity
with revm.

```sh
cargo run --manifest-path translator/Cargo.toml -- translate --op ADD
cargo run --manifest-path translator/Cargo.toml -- translate --op SUB
cargo test --manifest-path translator/Cargo.toml
```

## Calling convention

`evm_add` and `evm_sub` use the RISC-V integer calling convention: `a0` points
to operand A, `a1` to operand B, and `a2` to the destination. Each points to
four 8-byte-aligned `u64` limbs in little-endian limb order (least significant
first), using the target's native byte order. The destination may be exactly
the same address as either input, but must not otherwise overlap either 32-byte
input region. The inputs may overlap each other. The functions write four limbs
at `a2`; they have no register return value.

The functions read `a0`-`a2`, clobber caller-saved `t0`-`t5`, and preserve all
other registers. They use no stack. ADD detects unsigned overflow with `sltu`
for each addition and propagates carry. SUB computes `A - B`, detects the
borrow from `A - B`, subtracts the incoming borrow, and detects that second
borrow with `sltu`; the OR is propagated to the next limb. Each final carry or
borrow is discarded, giving arithmetic modulo 2^256. `li` and `ret` are GNU
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

The other path constructs EVM bytecode with `PUSH32`, the tested opcode,
`MSTORE`, and `RETURN` and executes it through revm using the fixed Cancun
hardfork. For SUB it pushes B first and then A, so the EVM stack evaluates
`A - B`. Each path produces all 32 result bytes independently; ADD compares
seven fixed cases and 1,000 deterministic randomized inputs, while SUB checks
nine directed cases (including wraparound and multi-limb borrows), 1,000
random pairs, and their reverse-operand counterparts, all with seed
`0x123456789abcdef0`.

Run from the repository root:

```sh
cargo test --manifest-path translator/Cargo.toml
cargo test --manifest-path translator/Cargo.toml --test rv64_execution -- --nocapture
```

The second command also prints an executed `ADD(5, 7)` example and its results.
The experiment checks only the arithmetic result modulo 2^256 and the required
SUB operand order. It does not verify complete EVM stack semantics, transaction
behavior, revert handling, other opcodes, or gas parity; the EVM bytecode
consumes gas, but the RV64 function does not implement gas metering. Passing
tests are experimental evidence, not a formal correctness proof or universal
EVM parity.

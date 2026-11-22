# Ferreth compiler

Ferreth compiles bounded, straight-line EVM bytecode to one RV64 guest.
The compiler supports `STOP`, `PUSH1`, `POP`, `DUP1`, `ADD`, and `SUB`.
The decoder reads through the first `STOP` or EOF and zero-pads a missing
`PUSH1` immediate. It also recognizes `PUSH2`–`PUSH32`, which the compiler
rejects. The guest has finite stack and image capacity, and fitting the
image reservation does not guarantee that the assembler can accept it.
Dynamic EVM control flow and gas metering are outside this subset.

Use `compiler::compile_evm_bytecode` to obtain assembly and machine code, or
`orchestrator::execute_guest_evm_bytecode` to compile and run from an empty
stack. The `execute_guest_evm_bytecode_with_initial_stack` variant accepts
caller-provided stack words. Generated RV64 code performs stack operations
and arithmetic. The host initializes the guest, runs it in `rvemu`, and
validates its published result under ADR-001.
[Guest linking architecture](../../docs/research/ferreth-guest-linking.md)

Run from the repository root using the [recorded toolchain and lockfile versions](../../README.md#getting-started).

```sh
cargo run --locked -p ferreth-compiler -- execute --bytecode 600560030100
cargo test --locked -p ferreth-compiler
```

## Isolated ADD and SUB routines

The original arithmetic experiment emits GNU-style assembly for ADD and SUB
modulo 2^256. Use `translate` to inspect the standalone pointer-based routines.

```sh
cargo run --locked -p ferreth-compiler -- translate --op ADD
cargo run --locked -p ferreth-compiler -- translate --op SUB
```

## Reproducible RV64 execution baseline

The `bench --op` command uses the isolated arithmetic differential harness.
It assembles the routines with `asm-rs`, executes them in `rvemu`, and compares
the complete 256-bit result with REVM under Cancun. The baseline measures
retired RV64 instructions. For machine-readable capture, check `dtc` before
running the command and validate the complete stdout stream.
[Benchmark stdout requirements](../../docs/runbooks/benchmark.md#json-stdout)

```sh
cargo run --locked --release -p ferreth-conformance -- \
  bench --op ALL --cases 1000 --seed 42 --format json \
  > runs/rv64-arithmetic-candidate.json
```

`ADD`, `SUB`, and `ALL` are accepted values for `--op`; `ALL` applies the same
shared directed vectors and the same seed-reset xorshift64 random operand pairs
to each operation. `--cases` is the number of random pairs per operation; the
JSON report separately identifies the 12 directed cases. A parity or execution
failure produces a nonzero status and writes JSON with operation, case index,
operands, available results, seed, and execution error evidence in the
operation's `failures` array.

If omitted, `--seed` defaults to `0x123456789abcdef0`; always retain the
reported seed with evidence so the exact random input sequence can be replayed.

The `instruction_categories` values are aggregate counts over successfully
executed cases. They are mutually exclusive and sum to `total_executed`.
Classification is performed on the decoded 32-bit instruction at rvemu's PC
immediately before execution, so assembler-expanded `li` and `ret` are counted
as their real instructions. The harness `EBREAK` return sentinel is excluded.
Unrecognized instruction encodings are counted as `other`; this baseline only
classifies base 32-bit RV64 encodings and would require extension if compressed
or other instruction formats were introduced.

The benchmark is provided by `ferreth-conformance`; the compiler crate itself
does not depend on REVM. The command writes its JSON report to stdout, so use a
new candidate path and do not overwrite an existing baseline file.

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

## Arithmetic differential tests

The `ferreth-conformance` arithmetic integration test assembles the generated
RV64 source with `asm-rs` and executes the machine code instruction by
instruction in `rvemu`. The harness
removes only the GNU metadata directives, which do not encode instructions.
The test initializes the emulator's memory and `a0`/`a1`/`a2` registers
directly, uses an `EBREAK` return address for deterministic termination, and
enforces a 256-instruction limit. The generated function uses RV64I
instructions, which are a subset of RV64IM. It does not use a software
fallback for RV64 execution.

The other path constructs EVM bytecode with `PUSH32`, the tested opcode,
`MSTORE`, and `RETURN` and executes it through revm 43 using the fixed Cancun
hardfork. The reference uses an in-memory contract and a validated call with
zero gas price and a 1,000,000 gas limit. For SUB it pushes B first and then
A, so the EVM stack evaluates `A - B`. Each path produces all 32 result bytes
independently; ADD compares
seven fixed cases and 1,000 deterministic randomized inputs, while SUB checks
nine directed cases (including wraparound and multi-limb borrows), 1,000
random pairs, and their reverse-operand counterparts, all with seed
`0x123456789abcdef0`.

Run from the repository root:

```sh
cargo test --locked -p ferreth-conformance
cargo test --locked -p ferreth-conformance --test rv64_execution -- --nocapture
```

The second command also prints an executed `ADD(5, 7)` example and its results.
The mixed-program tests additionally run seven directed sequences and 500
deterministic programs with 3-20 arithmetic operations, using seed
`0x123456789abcdef0`. Each random program contains both operations, and both
ADD-then-SUB and SUB-then-ADD orders are exercised. The test encodes PUSH32,
ADD and SUB as actual EVM bytecode; the RV64 path uses a host-managed virtual
stack and executes the existing assembled routine for every arithmetic
operation. The top item is the first operand, so `PUSH 3; PUSH 7; ADD;
PUSH 2; SUB` yields -8 modulo 2^256. Both paths return the top stack value,
even when other values remain underneath.

The library's `decode` function reports unsupported opcodes with byte offsets
and zero-pads truncated PUSH immediates. The sequence `6003600701601460050301`
decodes to `PUSH 3; PUSH 7; ADD; PUSH 20; PUSH 5; SUB; ADD`. The arithmetic
harness uses `execution::ProgramRunner` to execute it with a host stack and
generated RV64 arithmetic, yielding 2^256 - 5. The REVM comparison appends an
`MSTORE`/`RETURN` wrapper after decoding the original program. Those two opcodes
are outside the decoder's subset.

These tests exercise arithmetic composition and data flow, not translation of
EVM stack instructions into RISC-V or arbitrary EVM bytecode. The experiment
checks arithmetic results modulo 2^256 and the required SUB operand order. It
does not verify complete EVM stack semantics, transaction
behavior, revert handling, other opcodes, or gas parity; the EVM bytecode
consumes gas, but the RV64 function does not implement gas metering. Passing
tests are experimental evidence, not a formal correctness proof or universal
EVM parity. For the separate whole-program differential suite, see the
[conformance findings](../../docs/research/ferreth-evm-conformance.md).

# Ferreth CLI Runbook

## Overview

The `ferreth-compiler` CLI provides three entry points.

- `translate` emits assembly for standalone ADD and SUB arithmetic routines or
  an unlinked PUSH1 body.
- `decode` statically disassembles Cancun bytecode.
- `execute --bytecode` compiles and verifies one RV64 image, runs it as a
  guest-owned EVM frame, and reports the validated result and dynamic RV64
  instruction count. `execute --op` offers isolated guest adapters that use
  the same metered compiler and runner with an implicit `u64::MAX` budget.

The Rust Compiler (RC) path owns whole-program compilation. The historical
Rust Orchestrator (RO) `ProgramRunner` managed the stack in Rust and ran only
arithmetic in RV64. It remains available to historical experiments and
regression tests pending a separate removal milestone. The supported frame
API is [`execute_guest_evm_bytecode`](../../crates/ferreth-compiler/src/orchestrator.rs).

## Prerequisites

- Run commands from the Ferreth workspace root, where the root `Cargo.toml`
  declares the workspace member `crates/ferreth-compiler`.
- Use the [recorded Rust and Cargo environment](../../README.md#getting-started)
  for reproduction. The compiler uses Rust edition 2021.
- Cargo resolves and builds the Rust dependencies from the manifests,
  including `alloy-primitives`, `asm-rs`, and `rvemu`. REVM is a dependency of
  the separate `ferreth-conformance` package, not the compiler CLI. No separate
  RV64 assembler command is used by these CLI operations.

Pass `--locked` to Cargo to use the checked-in dependency versions.

```sh
cargo run --locked -p ferreth-compiler -- <command> [arguments]
```

## Command reference

The compiler CLI recognizes `translate`, `execute`, and `decode` as its top-level commands.
There is no dedicated `--help` option. Running the CLI without a recognized
command prints usage to stderr and exits 1.

### `translate`

| Invocation | Purpose and valid arguments | Output |
| --- | --- | --- |
| `cargo run --locked -p ferreth-compiler -- translate --op ADD` | Generate the arithmetic `ADD` RV64 assembly. `--op` must be `ADD` or `SUB` for arithmetic translation. | GNU-style `.text` assembly for the routine. |
| `cargo run --locked -p ferreth-compiler -- translate --op SUB` | Generate the arithmetic `SUB` RV64 assembly. | GNU-style `.text` assembly for the routine. |
| `cargo run --locked -p ferreth-compiler -- translate --op PUSH1 --value 5` | Generate a `PUSH1` opcode body. The immediate is an unsigned byte, `0..255`. Decimal and `0x`-prefixed values are accepted. | An unlinked opcode-body assembly listing and its 32-bit machine-code words. The listing includes a patchable overflow trampoline. |

`translate` does not accept other opcodes. The arithmetic translation prints
assembly only. The `PUSH1` translation also prints machine-code words.

### `execute`

| Invocation | Purpose and valid arguments | Output |
| --- | --- | --- |
| `cargo run --locked -p ferreth-compiler -- execute --op PUSH1 --value 5` | Run an isolated `PUSH1` guest operation. `--value` is `0..255` (decimal or `0x`-prefixed). | Linked executable assembly, machine-code words, final guest stack, and total/body/termination instruction counts. |
| `cargo run --locked -p ferreth-compiler -- execute --op ADD --a 5 --b 3` | Run isolated guest `ADD`. `--a` and `--b` are values parseable as EVM `U256` words. The initial stack is `[a, b]`. | Opcode, initial stack, final guest status, final stack, and dynamic RV64 instruction count. |
| `cargo run --locked -p ferreth-compiler -- execute --op SUB --a 5 --b 3` | Run isolated guest `SUB` with the same `U256` arguments and initial-stack order. `SUB` evaluates the top item minus the item below it (`b - a`). | For this example the result wraps to `2^256 - 2`; output includes the status, final stack, and dynamic count. |
| `cargo run --locked -p ferreth-compiler -- execute --op POP` | Run isolated guest `POP` with one initialized zero word. | Final status, empty final stack, and dynamic RV64 instruction count. |
| `cargo run --locked -p ferreth-compiler -- execute --op DUP1` | Run isolated guest `DUP1` with one initialized zero word. | Final status, duplicated stack, and dynamic RV64 instruction count. |
| `cargo run --locked -p ferreth-compiler -- execute --bytecode 600560035000 --gas 10` | Run a supported bytecode frame with an explicit budget. | Decoded instructions, final status, initial and remaining frame gas, gross gas used, final stack, and dynamic RV64 instruction count. A guest exception adds its name and code. |

`--op ADD` and `--op SUB` require both `--a` and `--b`. For the isolated
guest wrappers, `--a` is the lower stack item and `--b` is the top item. EVM
`SUB` evaluates `b - a`. Both operations use 256-bit modular arithmetic.

Both `execute` and `decode` accept zero or more pairs of ASCII digits
`0–9`, `a–f`, or `A–F` in `--bytecode`. Pass an empty string as `''`.
Odd lengths, whitespace, signs, prefixes such as `0x`, and non-ASCII
characters are invalid. During execution decoding, the first `STOP` ends
the program. Trailing bytecode text must still satisfy the hex grammar.
Missing trailing PUSH immediate bytes are zero-padded.

Place an optional `--gas` after the bytecode argument. The budget is an
unsigned 64-bit value in decimal or hexadecimal with a lowercase `0x`
prefix. For example, `--gas 2` and `--gas 0x2` are equivalent. Omitting
`--gas` uses `u64::MAX` (18446744073709551615).

```sh
cargo run --locked -p ferreth-compiler -- execute --bytecode 5a00 --gas 0x2
```

```text
Decoded instructions: [Gas, Stop]
Final status: Normal
Initial frame gas: 2
Remaining frame gas: 0
Gross gas used: 2
Final stack: [0]
Dynamic RV64 instruction count: 27
```

With `--gas 1`, the same bytecode reports:

```text
Decoded instructions: [Gas, Stop]
Final status: GuestException
Guest exception: OutOfGas (3)
Initial frame gas: 1
Remaining frame gas: 0
Gross gas used: 1
Final stack: []
Dynamic RV64 instruction count: 16
```

`--gas 0` is explicit. STOP and empty EOF complete normally with zero
remaining and zero used, with 10 and 9 RV64 instructions respectively.
With `5a00` and budget 0, GAS reports `OutOfGas (3)`, zero used, and 16
RV64 instructions. A normal frame can use its entire budget, as the
budget-2 example shows. All guest exceptions settle remaining gas to zero.
Gross gas used is the initial budget minus settled remaining gas. Dynamic
counts measure executed RV64 instructions, including the terminal publisher.

With budget 10, `600560035000` completes normally with remaining gas 2,
gross used 8, stack `[5]`, and 54 RV64 instructions. With budget 20,
`600101` reports `StackUnderflow (2)`, remaining gas 0, gross used 20,
stack `[1]`, and 37 RV64 instructions. With budget 3080, 1024 copies of
`6001` followed by `5a00` report `StackOverflow (1)`, remaining gas 0,
gross used 3080, 1024 unchanged stack words, and 17,429 RV64 instructions.

### `decode`

Statically disassemble EVM bytecode:

```sh
cargo run --locked -p ferreth-compiler -- decode --bytecode 6005600350
```

See the [EVM Bytecode Disassembler](decode.md) runbook for output details,
examples, and limitations.

The separate `ferreth-conformance` CLI provides arithmetic and guest benchmarks.
Check the `dtc` prerequisite before capturing JSON output.
[Benchmark stdout requirements](benchmark.md#json-stdout)

```sh
cargo run --locked -p ferreth-conformance -- bench --op ADD --cases 100 --format json
```

See the [Benchmark Runbook](benchmark.md) for supported options and report
details.

## Exit codes and diagnostics

| Exit | Outcome |
| --- | --- |
| 0 | A valid guest result, including `GuestException`, or a successful translation or disassembly. |
| 1 | Invalid CLI input, execution decoding failure, compilation or linking failure, or host execution or result-validation error. |

Invalid flags and numeric values print usage to stderr. Malformed bytecode
text prints `invalid bytecode hex` with an explanation. `execute` also prints
usage. Valid hex `02` fails execution decoding with the offending byte and
offset. Valid hex `610001` decodes as PUSH2 and fails at compilation.
Executor faults and RV64 instruction-limit failures report an error on
stderr. The RV64 limit is internal to the CLI.

## Supported scope

| Component | Supported opcodes |
| --- | --- |
| Canonical Cancun frame execution | `STOP`, `PUSH1`, `POP`, `DUP1`, `ADD`, `SUB`, `GAS` |
| Decoder used by canonical execution | Canonical set plus `PUSH2`–`PUSH32` |
| Isolated guest execution | `PUSH1`, `ADD`, `SUB`, `POP`, `DUP1` |
| Static disassembly | Broader Cancun opcode inventory, including bytes outside the execution subset |

Canonical execution meters fixed-cost Cancun frames. Transaction intrinsic
gas, dynamic gas pricing, refunds, jumps, memory, storage, and calls are
outside the milestone. [ADR-002](../adr/002-cancun-evm-gas-metering.md)

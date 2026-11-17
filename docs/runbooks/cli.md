# Ferreth CLI Runbook

## Overview

The `ferreth-compiler` package provides a small command-line interface for the
current EVM-to-RV64 experiment:

- `translate` displays RV64 assembly for `ADD`, `SUB`, or a `PUSH1` opcode body.
- `execute` runs isolated guest operations through RO (the isolated
  **R**untime **O**rchestrator), or compiles and runs supported EVM bytecode
  through RC (the whole-program **R**V64 **C**ompiler).
- Execution commands report the guest result and dynamically executed RV64
  instruction counts. Some commands also print the linked assembly and
  machine-code words.

RO executes one isolated opcode with the shared guest state and result
contract. RC decodes supported bytecode, links its RV64 bodies into one guest
program, and executes that program. Neither path implements general EVM
semantics or gas accounting.

## Prerequisites

- Run commands from the Ferreth workspace root, where the root `Cargo.toml`
  declares the workspace member `crates/ferreth-compiler`.
- Install Rust and Cargo. The package uses Rust edition 2021; the manifests do
  not declare a `rust-version` minimum.
- Cargo resolves and builds the Rust dependencies from the manifests,
  including `asm-rs`, `revm`, and `rvemu`. No separate RV64 assembler command is
  used by these CLI operations.

The command form used below is:

```sh
cargo run -p ferreth-compiler -- <command> [arguments]
```

## Command reference

The CLI has three top-level commands: `translate`, `execute`, and `bench`.
There is no dedicated `--help` option. Running the CLI without a recognized
command prints usage to stderr and exits unsuccessfully.

### `translate`

| Invocation | Purpose and valid arguments | Output |
| --- | --- | --- |
| `cargo run -p ferreth-compiler -- translate --op ADD` | Generate the arithmetic `ADD` RV64 assembly. `--op` must be `ADD` or `SUB` for arithmetic translation. | GNU-style `.text` assembly for the routine. |
| `cargo run -p ferreth-compiler -- translate --op SUB` | Generate the arithmetic `SUB` RV64 assembly. | GNU-style `.text` assembly for the routine. |
| `cargo run -p ferreth-compiler -- translate --op PUSH1 --value 5` | Generate a `PUSH1` opcode body. The immediate is an unsigned byte, `0..255`. Decimal and `0x`-prefixed values are accepted. | An unlinked opcode-body assembly listing and its 32-bit machine-code words. The listing includes a patchable overflow trampoline; it is not the complete linked executable. |

`translate` does not accept other opcodes. The arithmetic translation prints
assembly only; the `PUSH1` translation also prints machine-code words.

### `execute`

| Invocation | Purpose and valid arguments | Output |
| --- | --- | --- |
| `cargo run -p ferreth-compiler -- execute --op PUSH1 --value 5` | Run an isolated `PUSH1` guest operation. `--value` is `0..255` (decimal or `0x`-prefixed). | Linked executable assembly, machine-code words, final guest stack, and total/body/termination instruction counts. |
| `cargo run -p ferreth-compiler -- execute --op ADD --a 5 --b 3` | Run isolated guest `ADD`. `--a` and `--b` are values parseable as EVM `U256` words. The initial stack is `[a, b]`. | Opcode, initial stack, final guest status, final stack, and dynamic RV64 instruction count. |
| `cargo run -p ferreth-compiler -- execute --op SUB --a 5 --b 3` | Run isolated guest `SUB` with the same `U256` arguments and initial-stack order. `SUB` evaluates the top item minus the item below it (`b - a`). | For this example the result wraps to `2^256 - 2`; output includes the status, final stack, and dynamic count. |
| `cargo run -p ferreth-compiler -- execute --push1-sequence 5,3` | Run a comma-separated sequence of `PUSH1` immediates; each item must be a decimal or `0x`-prefixed byte value. | One linked executable's assembly, final stack, and total/opcode-body/termination counts. |
| `cargo run -p ferreth-compiler -- execute --bytecode 600560030100` | Decode, compile, and run even-length hexadecimal bytecode with no `0x` prefix. Supported source opcodes are `STOP`, `PUSH1`–`PUSH32`, `ADD`, and `SUB`; RC supports only `PUSH1` among the push widths. | Decoded instructions, final status, final stack, and dynamic RV64 instruction count. It does not print the complete compiled assembly or machine-code listing. |

`--op ADD` and `--op SUB` require both `--a` and `--b`. For the isolated
guest wrappers, `--a` is the lower stack item and `--b` is the top item; EVM
`SUB` therefore evaluates `b - a`. Both operations use 256-bit modular
arithmetic.

The bytecode argument is pairs of hexadecimal digits. It must not include
spaces or a `0x` prefix. A `STOP` ends decoding, so bytes after the first
`STOP` are not interpreted. Unlike bytecode execution, isolated `PUSH1`
commands print the generated RV64 assembly and machine-code words directly.

### `bench`

Benchmark command syntax and report fields are covered in the
[Ferreth Benchmark Runbook](benchmark.md). The supported forms are:

```sh
cargo run -p ferreth-compiler -- bench --op ADD --cases 1000 --seed 42 --format json
cargo run -p ferreth-compiler -- bench --op SUB --cases 1000 --seed 42 --format json
cargo run -p ferreth-compiler -- bench --op ALL --cases 1000 --seed 42 --format json
cargo run -p ferreth-compiler -- bench guest --format json
```

The arithmetic benchmark requires `--op ADD|SUB|ALL`, `--cases N`, and
`--format json`; `--seed N` is optional. The guest benchmark accepts
`--format json` (also accepted as `--format=json`).

## Important examples

### Translate ADD and SUB

```sh
cargo run -p ferreth-compiler -- translate --op ADD
cargo run -p ferreth-compiler -- translate --op SUB
```

Each command prints an RV64 assembly routine. The generated arithmetic routine
operates on four 64-bit limbs for a 256-bit value.

### Translate PUSH1 and inspect its instructions

```sh
cargo run -p ferreth-compiler -- translate --op PUSH1 --value 5
```

The output includes the readable opcode-body assembly followed by 11
machine-code words in hexadecimal. This is an unlinked body with a placeholder
overflow trampoline.

### Run isolated guest operations

```sh
cargo run -p ferreth-compiler -- execute --op PUSH1 --value 5
cargo run -p ferreth-compiler -- execute --op ADD --a 5 --b 3
cargo run -p ferreth-compiler -- execute --op SUB --a 5 --b 3
```

Representative results:

```text
PUSH1: Final stack: [5]
       Instructions: 17 (opcode body 10, termination 7)
ADD:   Final stack: [8]
       Dynamic RV64 instruction count: 49
SUB:   Final stack: [115792089237316195423570985008687907853269984665640564039457584007913129639934]
       Dynamic RV64 instruction count: 49
```

For SUB, `--a 5 --b 3` initializes `[5, 3]`, so the top item is subtracted
from the lower item as `3 - 5`, modulo 2^256.

### Run a PUSH1 sequence

```sh
cargo run -p ferreth-compiler -- execute --push1-sequence 5,3
```

This links and executes both pushes in one guest run. The expected stack is
`[5, 3]`, with 27 instructions: 20 in the two opcode bodies and 7 in shared
normal termination.

### Run a complete bytecode program ending in STOP

```sh
cargo run -p ferreth-compiler -- execute --bytecode 600560030100
```

Expected output fields include:

```text
Decoded instructions: [Push(5), Push(3), Add, Stop]
Final status: Normal
Final stack: [8]
Dynamic RV64 instruction count: 70
```

The explicit `STOP` is an executed guest instruction. The bytecode program
without STOP (`6005600301`) executes 69 RV64 instructions; with STOP it
executes 70.

## Error handling

- **Missing arguments or invalid CLI syntax:** prints the usage block to
  stderr and returns a failure status. Unsupported translation operations,
  missing flags, invalid numeric values, and malformed hex argument syntax
  follow this path.
- **Unsupported opcode or truncated PUSH in bytecode:** prints
  `bytecode decoding failed` with the offending byte offset and returns a
  failure status. For example, `02` is an unsupported opcode, and `600561` is
  a truncated `PUSH2`.
- **Decoded but uncompiled PUSH widths:** `PUSH2`–`PUSH32` are recognized by
  the decoder, but RC reports an unsupported instruction when asked to
  compile them. For example, `cargo run -p ferreth-compiler -- execute
  --bytecode 610001` is decoded and then rejected by compilation.
- **Guest stack underflow or overflow:** guest-detected stack errors are
  reported in `Final status` as `GuestException`, not as malformed CLI input.
  For example, `cargo run -p ferreth-compiler -- execute --bytecode 600101`
  leaves `[1]` on the stack and reports `GuestException` after 21 executed
  instructions. A program that tries to push beyond the 1024-word guest stack
  similarly takes the guest overflow path.
- **Executor or instruction-limit failure:** the CLI has no option to set an
  instruction limit. Limits are enforced internally; if execution fails
  before a valid guest result is published, the relevant command reports the
  error to stderr and returns a failure status. Arithmetic benchmark parity
  failures still emit their JSON evidence before returning failure.

## Current limitations

The decoder and compiler do not support the same instruction set:

| Component | Supported opcodes |
| --- | --- |
| EVM decoder | `STOP`, `PUSH1`–`PUSH32`, `ADD`, `SUB` |
| RC bytecode compiler | `STOP`, `PUSH1`, `ADD`, `SUB` |
| Isolated guest operations (RO) | `PUSH1`, `ADD`, `SUB` |

Other `PUSH` widths can decode but cannot be compiled to RV64 by RC. Control
flow other than stopping at `STOP`, EVM memory and storage operations, and gas
accounting are not implemented. Reaching the end of bytecode also completes
the compiled guest program; it does not imply support for arbitrary EVM
control flow or transaction behavior.

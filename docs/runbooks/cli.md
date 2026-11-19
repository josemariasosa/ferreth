# Ferreth CLI Runbook

## Overview

The `ferreth-compiler` package provides a small command-line interface for the
current EVM-to-RV64 experiment:

- `translate` displays RV64 assembly for `ADD`, `SUB`, or a `PUSH1` opcode body.
- `execute` runs isolated guest operations through RO (the isolated
  **R**untime **O**rchestrator), or compiles and runs supported EVM bytecode
  through RC (the whole-program **R**V64 **C**ompiler).
- `decode` statically disassembles EVM bytecode; it does not execute it.
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
  including `alloy-primitives`, `asm-rs`, and `rvemu`. REVM is a dependency of
  the separate `ferreth-conformance` package, not the compiler CLI. No separate
  RV64 assembler command is used by these CLI operations.

The command form used below is:

```sh
cargo run -p ferreth-compiler -- <command> [arguments]
```

## Command reference

The compiler CLI has three top-level commands: `translate`, `execute`, and `decode`.
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
| `cargo run -p ferreth-compiler -- execute --op POP` | Run isolated guest `POP` with one initialized zero word. | Final status, empty final stack, and dynamic RV64 instruction count. |
| `cargo run -p ferreth-compiler -- execute --op DUP1` | Run isolated guest `DUP1` with one initialized zero word. | Final status, duplicated stack, and dynamic RV64 instruction count. |
| `cargo run -p ferreth-compiler -- execute --push1-sequence 5,3` | Run a comma-separated sequence of `PUSH1` immediates; each item must be a decimal or `0x`-prefixed byte value. | One linked executable's assembly, final stack, and total/opcode-body/termination counts. |
| `cargo run -p ferreth-compiler -- execute --bytecode 600560035000` | Decode, compile, and run even-length hexadecimal bytecode with no `0x` prefix. Supported source opcodes are `STOP`, `PUSH1`–`PUSH32`, `ADD`, `SUB`, `POP`, and `DUP1`; RC supports only `PUSH1` among the push widths. | Decoded instructions, final status, final stack, and dynamic RV64 instruction count. It does not print the complete compiled assembly or machine-code listing. |

`--op ADD` and `--op SUB` require both `--a` and `--b`. For the isolated
guest wrappers, `--a` is the lower stack item and `--b` is the top item; EVM
`SUB` therefore evaluates `b - a`. Both operations use 256-bit modular
arithmetic.

The bytecode argument is pairs of hexadecimal digits. It must not include
spaces or a `0x` prefix. A `STOP` ends decoding, so bytes after the first
`STOP` are not interpreted. Unlike bytecode execution, isolated `PUSH1`
commands print the generated RV64 assembly and machine-code words directly.

### `decode`

Statically disassemble EVM bytecode:

```sh
cargo run -p ferreth-compiler -- decode --bytecode 6005600350
```

See the [EVM Bytecode Disassembler](decode.md) runbook for output details,
examples, and limitations.

REVM-backed arithmetic and guest benchmarks are provided by the separate
`ferreth-conformance` CLI. For example:

```sh
cargo run -p ferreth-conformance -- bench --op ADD --cases 100 --format json
```

See the [Benchmark Runbook](benchmark.md) for supported options and report
details.

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
  error to stderr and returns a failure status. Conformance benchmark parity
  failures emit their JSON evidence before returning failure.

## Current limitations

The decoder and compiler do not support the same instruction set:

| Component | Supported opcodes |
| --- | --- |
| `execute` bytecode decoder | `STOP`, `PUSH1`–`PUSH32`, `ADD`, `SUB`, `POP` |
| RC bytecode compiler | `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, `DUP1` |
| Isolated guest operations (RO) | `PUSH1`, `ADD`, `SUB`, `POP`, `DUP1` |

Other `PUSH` widths can decode but cannot be compiled to RV64 by RC. Control
flow other than stopping at `STOP`, EVM memory and storage operations, and gas
accounting are not implemented. Reaching the end of bytecode also completes
the compiled guest program; it does not imply support for arbitrary EVM
control flow or transaction behavior.

# EVM Bytecode Disassembler

## Overview

`decode` converts hexadecimal EVM bytecode into a readable instruction listing.
It is static disassembly: it does not execute EVM instructions or simulate or
display the EVM stack. Decoding an opcode does not mean Ferreth can compile it
to RV64.

## Usage

Run from the Ferreth workspace root:

```sh
cargo run -p ferreth-compiler -- decode --bytecode 6005600350
```

Provide one continuous sequence of whole-byte hexadecimal digits, with no
spaces or `0x` prefix. Upper- and lowercase hex are accepted. The only decode
option is `--bytecode HEX`.

## Examples

### Basic stack operations

Input `6005600350` represents:

```text
PUSH1 0x05
PUSH1 0x03
POP
```

The CLI prints:

```text
PC      BYTES       OPCODE
0x0000  6005        PUSH1 0x05
0x0002  6003        PUSH1 0x03
0x0004  50          POP
```

### Arithmetic

`600560030100` contains two pushes, `ADD`, and `STOP`:

```text
PC      BYTES       OPCODE
0x0000  6005        PUSH1 0x05
0x0002  6003        PUSH1 0x03
0x0004  01          ADD
0x0005  00          STOP
```

### Instructions after STOP

`6005006009` disassembles as:

```text
PC      BYTES       OPCODE
0x0000  6005        PUSH1 0x05
0x0002  00          STOP
0x0003  6009        PUSH1 0x09
```

The listing covers the entire byte stream. `STOP` ends ordinary EVM execution,
so the final `PUSH1` is unreachable during ordinary execution; a static
disassembler still displays it.

### PUSH immediates

`610102` is a `PUSH2` whose immediate is `0x0102`:

```text
PC      BYTES       OPCODE
0x0000  610102      PUSH2 0x0102
```

## Reading the output

- **PC** is the byte offset where the instruction starts, shown in hexadecimal.
- **BYTES** contains the exact instruction bytes, including any immediate.
- **OPCODE** shows the mnemonic and, for PUSH instructions, the immediate value.
- `PUSH1` occupies two bytes: its opcode byte (`60`) and one immediate byte.
  `PUSH2` occupies three bytes: its opcode byte (`61`) and two immediate bytes.
  Immediate bytes belong to that PUSH; they are not disassembled as separate
  instructions.

## Errors and limitations

- Malformed hex, including a `0x` prefix, non-hex characters, or an odd number
  of digits, prints `invalid bytecode hex` to stderr and exits unsuccessfully.
- A truncated `PUSH` is listed with the immediate bytes that are present and
  the number missing; it does not fail disassembly. For example, input `61`
  prints `PUSH2 (truncated: missing 2 immediate bytes)`.
- Defined Cancun opcodes are printed by mnemonic, including opcodes the RV64
  compiler does not support (for example, `02` is `MUL`). Undefined opcode
  bytes are displayed as `UNKNOWN (0xNN)`; for example, `0c` is
  `UNKNOWN (0x0c)`. Neither case prevents static disassembly.
- The disassembler names opcodes and reports byte spans only. It does not
  analyze control flow, gas, stack effects, or compiler support. The separate
  EVM-to-RV64 bytecode compiler currently supports only `STOP`, `PUSH1`, `ADD`,
  `SUB`, and `POP`; successful disassembly alone does not make other opcodes
  compilable.

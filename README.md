<p align="center">
  <img src="assets/ferreth-logo.png" alt="Ferreth Compiler logo" width="440">
</p>

<div align="center">

![Rust](https://img.shields.io/badge/language-Rust-orange?logo=rust)
![RISC-V](https://img.shields.io/badge/target-RISC--V-283272)
![EVM](https://img.shields.io/badge/source-EVM-627EEA)
![Status](https://img.shields.io/badge/status-experimental-yellow)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

# An experimental EVM-to-RISC-V compiler.

*Built for open architectures, reproducible research, and hardware exploration.*

</div>

## Why Ferreth?

We believe the future of computing should be built on **open architectures**.

Ferreth explores whether Ethereum Virtual Machine (EVM) bytecode can be compiled into RISC-V instructions while preserving EVM execution semantics.

Our long-term ambition is to investigate a universal EVM-to-RISC-V compilation approach, independent of any particular processor implementation, emulator, or hardware platform.

By combining the EVM's execution model with RISC-V's open instruction set architecture, we hope to make Ethereum-oriented execution more accessible to researchers, compiler engineers, and hardware enthusiasts.

This is experimental research, not a production-ready EVM implementation.

## How It Works

```text
          EVM Bytecode
                |
                v
         Ferreth Compiler
                |
                v
          RISC-V RV64
                |
                v
       RISC-V Execution
                |
                v
           Guest State
                |
                v
       Compare with REVM
```

Ferreth currently compiles a small subset of EVM instructions into executable RV64 machine code, runs them through `rvemu`, and validates their behavior against `revm`.

The architecture separates compilation from execution through a backend-neutral interface, allowing future experiments with other emulators or native RISC-V hardware.

## Current Status

Ferreth is in early development.

| Component | Status |
|---|---|
| EVM bytecode decoding | Partial |
| RV64 code generation | Experimental |
| STOP, PUSH1, ADD, SUB | Implemented |
| Guest stack and runtime ABI | Implemented |
| Whole-program RV64 execution | Implemented for supported opcodes |
| Differential validation against REVM | Implemented for initial subset |
| Additional EVM opcodes | Planned |
| Alternative RISC-V backends | Future research |
| RTL and hardware acceleration | Future research |

The initial implementation includes deterministic directed and randomized differential tests against REVM.

See [ADR-001](docs/adr/001-evm-guest-state-abi.md) for the runtime architecture.

## Getting Started

Requires Rust and Cargo.

Clone the repository and run the test suite:

```bash
cargo test --workspace
```

Execute a simple EVM program:

```bash
cargo run -p ferreth-compiler -- execute --bytecode 600560030100
```

This program computes `5 + 3` using EVM `PUSH1`, `ADD`, and `STOP` instructions, producing a final stack containing `8`.

Explore the generated RV64 code:

```bash
cargo run -p ferreth-compiler -- translate --op ADD
```

Run the arithmetic benchmarks:

```bash
cargo run -p ferreth-compiler -- \
  bench --op ALL --cases 1000 --seed 42 --format json
```

For more commands and examples:

- [CLI Runbook](docs/runbooks/cli.md)
- [Benchmark Runbook](docs/runbooks/benchmark.md)

## Research Directions

**EVM-to-RISC-V Compilation**

Expand opcode coverage, improve code generation, and investigate EVM semantic compatibility across different RISC-V execution environments.

**Conformance and Verification**

Develop reproducible differential tests and validation tools to compare compiled execution against established EVM implementations.

**Open Hardware Exploration**

Investigate native RISC-V execution, FPGA implementations, RTL designs, and potential Ethereum-oriented hardware acceleration.

No performance or efficiency advantage is assumed; these are questions to be investigated experimentally.

## Contributing

Ferreth is an open research effort, and contributions are welcome.

Whether you are interested in Rust, compilers, Ethereum, RISC-V, emulators, verification, or digital hardware design, there is room to explore.

You can contribute by improving opcode coverage, adding differential tests, reviewing architecture decisions, investigating execution backends, improving documentation, or proposing experiments.

Open an issue to discuss an idea, report a bug, or propose a research direction. Pull requests are welcome.

**You don't need to be an EVM or RISC-V expert to contribute. Curiosity and a willingness to experiment are enough.**

## License

Licensed under either of:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.

---

*Ferreth — Exploring Ethereum execution through open instruction sets.*

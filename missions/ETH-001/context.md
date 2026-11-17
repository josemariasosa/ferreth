# ETH-001 Context: 256-bit ADD/SUB

This short document provides offline background only. It is not the executable mission specification and does not approve ports, operation encodings, or acceptance criteria.

- EVM words are 256 bits. `ADD` and `SUB` operate modulo \(2^{256}\); overflow and underflow wrap.
- The hardware component is combinational: it has no clock or reset.
- Ethereum opcode-level stack behavior and gas semantics are outside this component's scope. Ethereum protocol rules define gas; hardware latency does not.
- RV64 has 64-bit integer registers, but ETH-001 does not implement a CPU or require a limb-based interface.
- [Verified-zkEVM/evm-asm](../../specs/references/evm-asm.md) is a related software and formal-methods research reference, not a hardware implementation. Its evidence pack is offline material, not proof of current upstream status.

The executable mission must define the authorized files, exact interface, and acceptance criteria before implementation.

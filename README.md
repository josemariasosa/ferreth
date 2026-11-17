# ferrum-eth

Exploratory research into EVM-to-RISC-V behavioral parity and Ethereum-oriented hardware acceleration. 

## Research Tracks

1. **EVM-to-RISC-V translation and behavioral parity** - Begin with small EVM bytecode sequences translated to RV64IM and compare observable behavior with a pinned executable reference. Yul is a possible later path, with its compiled EVM bytecode explicitly identified.
2. **Ethereum-oriented hardware acceleration** - Compare conventional RV64IM implementations with specialized RTL or accelerators under defined workloads; no performance or efficiency advantage is assumed.



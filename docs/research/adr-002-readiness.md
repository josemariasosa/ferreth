# ADR-002 readiness at v0.1.0

**Ready with constraints.** Source inspection of clean main at 99544a588b4b6edabd74292db51b4a13b4f8488a (the v0.1.0 tag) identified no P0 engineering blocker. Maintainer approval of the proposed ABI and diagnostics is the remaining decision. Skip a preparatory engineering phase.

| Boundary | Source observation | Implementation implication |
|---|---|---|
| Decode and dispatch | [decode_detailed](../../crates/ferreth-compiler/src/lib.rs) stops at first STOP and pads truncated PUSH. [symbolic::compile](../../crates/ferreth-compiler/src/compiler/symbolic.rs) lowers STOP, PUSH1, POP, DUP1, ADD and SUB. Wider PUSHes are rejected. | Add GAS to execution decode and continuation naming. |
| Code generation | [Arithmetic wrapper](../../crates/ferreth-compiler/src/rv64/opcodes/arithmetic.rs) checks depth before calling the local leaf. Individual stack fragments guard before writes. | Insert inline charging ahead of guards. Preserve leaf contracts. |
| Linking | compose, assemble_image and verified_program resolve local names and publisher roles. [verify_final_image](../../crates/ferreth-compiler/src/rv64/verify.rs) independently knows fixed word layouts. | Local branch and distant JAL suffice. New declarations alone cannot authorize changed layouts. |
| ABI and executor | [initialize_guest_state and read_guest_result](../../crates/ferreth-compiler/src/rv64/guest.rs) use 24 bytes in 4 KiB. [execute_raw_program](../../crates/ferreth-compiler/src/orchestrator.rs) initializes stack, runs rvemu to a publisher then EXIT_PC, and validates publication. | Extend metadata and publishers. Keep host faults distinct. |
| Differential boundary | [Existing integration tests](../../crates/ferreth-conformance/tests/rc_revm_conformance.rs) use a transaction budget. Direct REVM accepts frame gas. | Keep transaction tests and use the [frame corpus](adr-002-reference-corpus.md) for gas. |

P1 work consists of budget injection, code generation, ABI validation and conformance coverage. P2 includes reconciliation of release-era evidence. The [historical cutover](../../research/guest-linking/phase-2.4-public-cutover.md) recorded 212 passing and three ignored tests plus separate passes for capacity probes. The read-only assessment did not run a fresh workspace suite. P3 includes alternative backends and optimization. rvemu 0.0.11 is the active executor. The bounded image (1,048,572 bytes) and assembler label limits constrain capacity after metering. No composer redesign or alternate backend is required by the evidence.

The original frame-oracle and ordering questions are addressed by RF-001–RF-007 and the subsequent paired experiment. The [conformance findings](ferreth-evm-conformance.md) record existing regressions and the [TDD roadmap](adr-002-tdd.md) maps the pending gas regressions to implementation phases. The reference investigation establishes expected behavior. Ferreth gas execution remains unverified.

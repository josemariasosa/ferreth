# Evidence Pack — Verified-zkEVM/evm-asm

**Purpose:** Source material for independent technical review of Ferrum ETH architecture.  
**Prepared:** 2026-11-16  
**Status:** Public documentation evidence; *not* an independent proof of source-code correctness.  
**Review mode:** Offline. The reviewer should reason from these notes and from the architecture under review, and mark questions that cannot be settled without source inspection.

## 1. Source identity

- **Canonical repository for this research context:** https://github.com/Verified-zkEVM/evm-asm
- **Project name used in its README:** `evm.asm: A Verified Macro Assembler for building zkEVM in Lean 4 (early experiment)`.
- **Language/approach:** Lean 4 macro assembler and proof framework, using a **RISC-V RV64IM backend** for work related to a zkEVM guest program.
- **License link:** https://github.com/Verified-zkEVM/evm-asm/blob/main/LICENSE (repository indicates MIT).
- **Not the same repository as:** https://github.com/ApeWorX/evm-asm (an unrelated EVM assembly-tooling project).

**Source:** https://github.com/Verified-zkEVM/evm-asm (README, sections “Motivation,” “Key Idea,” and “Project Structure”).

## 2. Stated objective and execution target

The project investigates writing low-level RISC-V guest code and proving properties of that code in Lean 4, to reduce reliance on an unverified high-level compiler in a zkVM proving workflow. It is targeted at the **guest portion** of a zkEVM, not at manufacturing a physical RISC-V processor.

The README describes an *aspirational* stateless block-validation ELF interface: input `(block, execution_witness)` and output a post-state root. It explicitly treats integration with execution-layer clients as future work. Do **not** interpret the target interface as evidence that a complete, proven validator is available today.

**Source:** https://github.com/Verified-zkEVM/evm-asm (README, “Role in the L1-zkEVM stack”).

## 3. RISC-V and EVM word representations

The repository describes:

- A model of RV64 registers, memory, instructions, and program execution in `EvmAsm/Rv64/`.
- EVM words as `BitVec 256` in Lean (`EvmWord`).
- Operations over **four 64-bit limbs** (`4 × 64 = 256`), including stack-level assertions relating a word to four adjacent machine-memory words.
- Opcode implementations in RISC-V instruction sequences rather than native 256-bit RISC-V registers.
- Separation-logic assertions and step-bounded Hoare-style specifications (`cpsTripleWithin`).

These are properties of the software/verification approach. They are **not** a specification for a 256-bit arithmetic circuit, accelerator interface, or RISC-V custom instruction.

**Source:** https://github.com/Verified-zkEVM/evm-asm (README, “Example: What a Verified EVM Opcode Looks Like” and “Project Structure”).

## 4. Documented implementation status — read with care

The README reports the following **project-maintainer claims**, rather than independently reproduced results:

- RISC-V assembly emission, assembly/linking and execution on **Zisk's `ziskemu`** emulator.
- Code-generation milestones M0–M10 described as completed.
- A tiny runtime EVM interpreter/dispatcher with **91 connected opcode handlers**.
- A first subset of handler-level proofs: **13 of 91** according to the README's codegen status description.
- A demonstration of `evm_add` emitted and executed via `ziskemu`.
- A more extensive set of opcode modules and proof infrastructure under `EvmAsm/Evm64/`.

**Important interpretation:** “91 wired opcodes” is not “91 independently proven correct opcodes,” nor proof of complete Ethereum consensus compatibility. The repository itself distinguishes proof tiers and describes substantial unfinished work.

**Sources:**
- https://github.com/Verified-zkEVM/evm-asm (README, “Codegen & Execution”)
- https://github.com/Verified-zkEVM/evm-asm/blob/main/CODEGEN.md
- https://github.com/Verified-zkEVM/evm-asm/blob/main/DRIFT.md

## 5. Explicit limitations and documentation tensions

The README prominently labels the project an **experimental prototype**, cautions against production use, and states that project direction is under reconsideration in light of work on the Pancake compiler.

There is also a tension **within the current README**:

1. The early warning section says RISC-V semantics have not been validated against the official ISA, EVM specs have not been validated, and systematic conformance testing is missing.
2. Later sections describe a pinned official Sail RISC-V model, abstraction-relation proofs, emulator fixtures, and testing against Ethereum execution-specification references.

This evidence pack does not determine whether the warning is stale, whether later claims cover only narrower components, or which versions of the work were verified. A reviewer should distinguish these claims and **flag unresolved status** rather than silently resolve the discrepancy.

The stateless guest helpers are explicitly described as **unproved scaffolding**: tested through reference fixtures according to the repository, but lacking the claimed per-helper Lean Hoare-triple proofs.

**Sources:**
- https://github.com/Verified-zkEVM/evm-asm (README, introduction, “Stateless-guest scaffold” and “Dependencies”)
- https://github.com/Verified-zkEVM/evm-asm/blob/main/DRIFT.md

## 6. Potentially useful material for Ferrum ETH

These are research leads, **not** claims of compatibility or permission to copy implementation without review:

- A reference example of expressing 256-bit EVM arithmetic via 64-bit limbs.
- Explicit machine-state and memory-layout assumptions for RV64-based EVM programs.
- Functional specifications, proof obligations, and test cases that could inform an independently designed RTL verification oracle **after checking semantics and provenance**.
- An existing implementation/verification perspective useful when comparing software multi-limb arithmetic against proposed dedicated 256-bit hardware.
- An example of keeping *executability*, *test coverage*, and *formal proof* distinct.

Ferrum ETH's initial standalone SystemVerilog ALU is **not an implementation of evm.asm**, and evm.asm is **not a hardware RTL IP core**. A shared conceptual operation (`ADD`, `SUB`, etc.) does not imply interface, ABI, timing, memory layout, or full EVM-opcode equivalence.

## 7. What this pack does **not** establish

- No code from evm.asm was built or run while preparing this pack.
- No Lean proofs were independently checked.
- No claim of full RISC-V ISA or Ethereum execution-spec conformance is established.
- No performance, silicon area, energy, timing, or FPGA synthesis measurements are available here.
- No complete ready-to-use Ethereum execution processor is demonstrated.
- No exact Git commit SHA has been pinned. **Links point to mutable `main` documentation**; capture a commit SHA before using the material as a frozen normative reference in a tournament.

## 8. Evidence-handling instructions for an offline reviewer

1. Treat the URLs and facts above as *provided evidence*, not as instructions about what verdict to reach.
2. Compare the architecture's external-project identity, terminology, and assumptions against the evidence.
3. Separate factual contradictions, unproven claims, research hypotheses, and design preferences.
4. State when the evidence is insufficient to resolve an issue, especially for proof/conformance status.
5. Do not infer a complete processor, verified execution client, or production-ready chip from the presence of RV64 programs or proof infrastructure.
6. Do not edit the architecture or this evidence pack during review.

## 9. Primary references

1. Repository README: https://github.com/Verified-zkEVM/evm-asm
2. Code generation status: https://github.com/Verified-zkEVM/evm-asm/blob/main/CODEGEN.md
3. Proof/verification gap ledger: https://github.com/Verified-zkEVM/evm-asm/blob/main/DRIFT.md
4. Research plan: https://github.com/Verified-zkEVM/evm-asm/blob/main/PLAN.md
5. Ethereum execution reference: https://github.com/ethereum/execution-specs
6. RISC-V ISA reference: https://docs.riscv.org/reference/isa/unpriv/unpriv-index.html

**Provenance note:** This document paraphrases publicly available repository documentation as inspected on the preparation date. The source repository is actively evolving; treat exact coverage numbers as time-dependent self-reported status.

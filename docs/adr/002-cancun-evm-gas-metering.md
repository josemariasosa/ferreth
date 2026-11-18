# ADR-002: Cancun EVM Gas Metering and Guest Execution Accounting

- **Status:** Proposed
- **Date:** 2026-11-17
- **Specification target:** Ethereum Cancun (Dencun execution fork); Cancun rules as defined by the execution-specifications Cancun fork and applicable EIPs, with REVM `43.0.3` configured to `SpecId::CANCUN` as the initial differential reference.
- **Implementation:** Not implemented. This ADR authorizes no code changes.

## 1. Context

Ferreth compiles supported EVM bytecode to one linked RV64 guest program. Its accepted runtime contract is in [ADR-001](001-evm-guest-state-abi.md): the guest owns the live EVM stack in memory, `s0`/`s1` are the stack base/next-free pointers, and guest code publishes a result before reaching an executor-observed exit PC. Rust loads code, initializes guest state, executes the guest through `Rv64Executor`, and reads the published result. The guest program has no Rust callback per EVM opcode.

ADR-001 explicitly leaves gas accounting for a later decision. The current result record stores status, exception code, and final `s1`; it contains no gas fields. Current RV64 benchmarks count successfully executed guest instructions, and explicitly do not claim to measure EVM gas.

The RC compiler currently supports `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, and `DUP1`. The decoder recognizes `STOP`, `PUSH1`–`PUSH32`, `ADD`, `SUB`, `POP`, and `DUP1`; the broader decode set is not a compiled guest set. Empty bytecode falls through to a normal publisher, and explicit STOP transfers to that publisher. The result type currently distinguishes normal completion and guest exception; executor faults and instruction-limit exhaustion are separate host errors.

## 2. Problem Statement

Define a gas-accounting architecture that preserves ADR-001's autonomous guest execution and terminal result contract while supporting exact Cancun EVM execution-gas semantics, deterministic out-of-gas behavior, observable remaining gas, and future dynamic pricing. The proposal must distinguish Cancun EVM gas from transaction intrinsic gas and from RV64 execution metrics.

## 3. Existing Architectural Constraints

Established by accepted ADR-001 and current implementation:

1. **Guest execution autonomy:** RC emits one linked program; Rust does not dispatch each opcode during execution (`compiler.rs::compile_evm_bytecode`, `orchestrator.rs::execute_linked_program`).
2. **Guest state:** `s0` is the stack base and `s1` is next-free; depth is derived from pointers. The EVM stack is ascending in guest memory and separate from native `sp` (`rv64/guest.rs`, `rv64/memory.rs`).
3. **Register contract:** `s0`/`s1` are reserved. Current arithmetic cores use `a0`–`a2`, `t0`–`t5`, and preserve `s0`/`s1` (`rv64/add.rs`, `rv64/sub.rs`; ADR-001 §5). ADR-001 does not reserve a gas register.
4. **Completion contract:** normal and guest-exception paths publish metadata and transfer to the sentinel. The executor stops before fetching the sentinel (`compiler.rs` termination publishers; `rv64/executor/rvemu.rs::execute_until`; `rv64/guest.rs::read_guest_result`).
5. **Host responsibilities:** load/init/read guest state, bound execution, classify executor-level failures, and collect backend metrics. The `Rv64Executor` interface has no per-opcode callback (`rv64/executor/mod.rs`).
6. **Memory map:** code, metadata, EVM stack, unassigned space, and native stack are mapped in `rv64/memory.rs`; metadata currently reserves 24 bytes at its beginning. Any persistent guest gas field would require a reviewed ABI/layout extension.
7. **Opcode composition:** opcode generators provide stack-aware bodies with explicit patchable exceptional branches; the linker composes bodies and termination publishers (`rv64/{push,add,sub,pop,dup}.rs`, `compiler.rs`).
8. **Initial opcode subset:** RC compiled subset is `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, `DUP1`. No memory, storage, environment, call, or GAS instruction is compiled today.
9. **Conformance reference:** `execution.rs::run_evm_bytecode` configures REVM `SpecId::CANCUN`; `Cargo.toml` pins `revm = 43.0.3`; `tests/rc_revm_conformance.rs` compares RC execution with REVM for the implemented subset. Existing tests do not establish gas parity.
10. **Metrics:** `benchmark.rs` and `docs/runbooks/benchmark.md` report RV64 executed instructions by scope, not EVM gas. Keep those measurements distinct.

No accepted ADR establishes gas storage location, gas register assignment, gas-bearing ABI, stable exception-code values, or gas validation ordering.

## 4. Decision Drivers

- Preserve a single guest-owned EVM execution model and ADR-001 termination protocol.
- Match Cancun rules for every implemented opcode and make unsupported rules explicit.
- Make remaining gas observable to `GAS` and host results without host/guest disagreement.
- Preserve stack and memory invariants on failed instructions.
- Support dynamic charges, memory expansion, warm/cold accesses, refunds, and nested frames without changing the fundamental execution model.
- Keep gas units, guest instruction metrics, and executor errors separate.
- Permit exact REVM differential testing of statuses, state, and gas fields.

## 5. Cancun Semantic Baseline

The fork target is Cancun, not an unspecified “current EVM.” Use the Cancun branch of the Ethereum execution specifications and Cancun EIPs as the normative references, and REVM `43.0.3` with `SpecId::CANCUN` as a pinned executable oracle. Any source/spec update must identify whether it changes Cancun semantics or merely implementation details.

The initial RC opcode subset has fixed execution costs under Cancun. These values are cross-checked against the Cancun execution-specification gas constants and the Cancun REVM fork; the mapping is:

| Opcode | Cancun execution cost | Source basis |
|---|---:|---|
| STOP | 0 | Cancun gas schedule / opcode definition |
| ADD | 3 | Cancun gas schedule |
| SUB | 3 | Cancun gas schedule |
| PUSH1 | 3 | Cancun gas schedule |
| POP | 2 | Cancun gas schedule |
| DUP1 | 3 | Cancun gas schedule |

These are EVM execution gas units. There is no transaction intrinsic-gas model in Ferreth's current bytecode execution API. Intrinsic transaction gas (calldata, transaction base, access lists, creation/initcode and related transaction rules) belongs to a transaction-validation/host layer and must not be silently charged as opcode execution gas.

For a fixed-cost opcode, successful execution requires remaining gas at least equal to the cost; equality succeeds and leaves zero. If remaining gas is less than the required cost, the opcode causes OutOfGas before its operation takes effect and exceptional execution consumes the frame's remaining gas (reported as zero remaining at the failed frame). Cancun exceptional-halt semantics and REVM's exact error/output representation must be checked per case; a generic guest exception code is not sufficient as a gas conformance assertion.

An opcode whose own semantic validation fails can have precedence interactions with gas charging. Cancun interpreter rules commonly charge gas before operation-specific effects, but stack underflow/overflow is an exceptional condition with its own validation path. The order observed for all relevant Cancun cases must be established directly from the Cancun specification and pinned REVM; this ADR does not assert an ordering where evidence is not yet checked. In particular, a failed operation must not partially alter the live stack, memory, or persistent state, and an OOG result must be distinguishable from stack errors.

`GAS` (0x5a) reports gas remaining as observed at its opcode execution point. The current compiler does not support GAS. When added, its own charge/observation ordering must use Cancun behavior; the returned value must correspond to the correct remaining amount after the applicable GAS opcode charge, not to a host snapshot taken between opcodes.

Static costs are only the starting point. Memory opcodes charge for memory expansion according to Cancun's word-based memory-cost function; KECCAK256 adds per-word cost; storage and account access have warm/cold costs (EIP-2929) and SSTORE net-metering/refund behavior (EIP-2200 and EIP-3529); calls and creation have dynamic costs, forwarding rules, child-frame gas, and exceptional behavior. Refunds are a transaction-level counter applied under Cancun rules (including the one-fifth cap from EIP-3529); they do not replenish execution gas mid-frame and should not be conflated with gross execution gas consumed.

## 6. Considered Alternatives

### A. Host-managed gas accounting / host opcode dispatch

Rust would debit gas while selecting or coordinating each guest opcode.

- **ADR compatibility:** Conflicts with ADR-001's one guest program without per-opcode Rust callbacks if host dispatch becomes necessary for each step.
- **Autonomy and correctness:** Host can maintain a simple gas value, but must observe every executed opcode and branch. It cannot reliably meter arbitrary guest control flow without an instruction-level callback or an alternate interpreted execution loop. A host-only estimate risks disagreement with the actual guest path.
- **Halts and ABI:** Host could report OOG, but guest termination and partial guest state would need a new coordination protocol; callback errors could bypass guest result publication.
- **Registers/helpers/performance:** Low guest register pressure, high host/emulator coupling and likely callback overhead. Current executor abstraction exposes no opcode callback.
- **Dynamic gas/extensibility:** Host can consult context/state, but dynamic costs require reliable per-opcode guest-to-host handshakes and frame state. This becomes a competing execution model.
- **Debugging/testing/migration:** Easy to instrument a reference interpreter; hard to prove correspondence with generated control flow. Not recommended as Ferreth's execution architecture.

### B. Guest-managed per-opcode gas checks in generated code

Each generated opcode body checks and deducts its cost before performing the opcode.

- **ADR compatibility:** Preserves compiled single-run guest execution and sentinel publication.
- **Autonomy/correctness:** Directly accounts for actually executed guest operations, including control flow, if every body uses a common metering contract.
- **Halts/ABI:** OOG needs a guest exceptional publisher carrying an OOG classification and zero remaining gas; successful STOP/fallthrough publish remaining and consumed gas.
- **Registers/helpers:** Could tempt a reserved register, but no such register is currently free by architectural contract. A register-only design competes with scratch conventions and helpers.
- **Dynamic gas:** Fixed costs are straightforward; dynamic costs require shared guest helpers/state and explicit preflight/commit ordering.
- **Debugging/testing/performance:** Easy to correlate a charge with an opcode and compare boundary cases. Adds instructions per executed opcode; actual overhead is unmeasured.
- **Migration:** Touches each opcode generator and terminal publisher; preserves the compiler's architecture if gas state placement is settled separately.

### C. Guest-managed gas in memory-backed execution state

Guest gas is held in a defined execution-state record in guest memory, accessed by generated guest code or shared guest helpers. The record can include gas limit/remaining and later frame/context fields. A hot local copy may be used only if it is flushed consistently at every observable boundary and exceptional path.

- **ADR compatibility:** Best fits ADR-001's guest-memory state/result model, but adding ABI fields is an explicit extension to the accepted 24-byte result record/layout and therefore requires maintainer review.
- **Autonomy/correctness:** The guest remains authoritative and sees its own remaining gas; guest code can check/debit before each operation.
- **Halts/ABI:** Can publish exact terminal gas fields, but requires a versioned or extended result schema and defined precedence/atomicity for result writes.
- **Registers/helpers:** Avoids reserving a register; memory accesses add overhead. A cached register may be an optimization only after correctness and ABI review. Runtime helpers must share the same gas-state pointer/convention and not clobber stack pointers.
- **Dynamic gas:** Most extensible option: room for memory high-water mark, warm-state handles, refund counter, and nested frame budgets, without requiring these mechanisms in the first implementation.
- **Debugging/testing/performance:** Strong observability and simple host inspection; repeated memory loads/stores may be expensive. Cache/flush policy becomes an optimization invariant.
- **Migration:** Requires coordinated result-ABI and initialization changes; future-compatible if designed as an explicit versioned record.

### D. Basic-block gas accounting

Generated code charges a precomputed total at block entry or exit.

- **ADR compatibility:** Can preserve guest autonomy but adds compiler/linker control-flow obligations.
- **Correctness:** Exact only when every path has identical known cost and no opcode exposes intermediate gas. Dynamic costs, GAS, exceptional behavior, and externally visible side effects require split points or deoptimization. A failed opcode must not be charged as if later block instructions ran.
- **Failure/ABI:** OOG within a block must identify the precise failing opcode and preserve exact side-effect ordering; a block-total check may reject a program whose actual executed prefix would fit.
- **Register/helper/performance:** Fewer checks in simple straight-line code, more compile-time bookkeeping and potentially expensive slow paths. It does not reduce EVM gas semantics, only metering instruction overhead.
- **Extensibility/testing/migration:** Optimizations need a per-opcode semantic reference and differential traces. Likely to be invalidated by GAS, dynamic gas, branches, calls, and precise exception reporting unless blocks split aggressively.
- **Status:** Not suitable as the correctness baseline. Potential future optimization only after per-opcode guest metering exists and proof/validation maintains equivalent observable behavior.

## 7. Proposed Direction (Not Yet Accepted)

Prefer a guest-authoritative, memory-backed gas state, with a common guest metering interface invoked by each opcode body. The host supplies the initial execution gas limit and initializes the state before entry; generated code charges gas and determines OOG; the guest publishes final gas accounting with the existing terminal status/exception result; Rust validates and reports the published values. This recommendation is grounded in ADR-001's autonomous guest program, memory-backed guest state, and absence of any gas-register reservation. It is a direction for maintainer review, not an accepted ABI change.

The minimum conceptual state is `gas_limit` and `gas_remaining` in EVM gas units. Future design must reserve a place/interface for gross gas used, refund counter, and frame-local budget/context, but their encoding and lifecycle remain open. Gross gas consumed during execution is `gas_limit - gas_remaining` for a top-level execution frame absent any pre-execution intrinsic charge; refunds are reported separately and applied only where the relevant transaction-level contract requires it. Never derive EVM gas from RV64 instruction counts.

Metering invariants proposed for review:

1. Gas values are unsigned EVM gas units and are independent of RV64 instruction counts.
2. No EVM opcode's semantic effect begins unless its full required charge is available.
3. A charge is checked without underflow; equality succeeds, insufficient gas produces OOG.
4. OOG and other exceptional halts follow Cancun frame gas-consumption and state rollback rules; terminal result must not label a stack error as OOG or vice versa.
5. On successful termination, report the exact gas limit, gross execution gas used, and remaining gas for the defined execution scope. Refund counter/applied refund are separate fields when implemented.
6. `GAS` observes the Cancun-defined remaining amount at its execution point.
7. Host reports the guest-published accounting values and validates internal arithmetic consistency; it does not independently meter or overwrite guest results.
8. Given identical bytecode, initial stack, gas limit, fork, state, and execution context, gas results are deterministic.
9. Gas charges and terminal publication do not corrupt `s0`, `s1`, live stack words, memory state, or control flow.
10. Dynamic charges use the same charge/failure contract as static charges, with all relevant values calculated before irreversible effects.
11. Any batching/cache optimization preserves per-opcode observable gas, failure point, state and result equivalence.
12. Transaction intrinsic gas, execution gas, refund accounting, and RV64 metrics remain separately named and reported.

The precise boundary between charge validation and stack validation, and whether static gas is checked before stack preconditions in each implementation path, is unresolved pending source-level Cancun/REVM verification. Do not freeze an ordering from this proposal.

## 8. ABI and Compatibility

ADR-001's `GuestMetadata` is a 24-byte record of status, exception code, and final `s1` (`rv64/guest.rs`). Extending it in place changes the accepted ABI. A separate gas record within metadata memory, an enlarged/versioned record, or another ABI mechanism are viable. This ADR does not choose the byte offsets, width, alignment, versioning, compatibility reader, or whether final gas values are published before or after the status commit field.

Any accepted design must retain ADR-001's ordered publication contract: final stack pointer and all result data must be written before terminal status is treated as valid, and the guest must transfer to the exit sentinel. OOG must use that same deliberate termination path, not an emulator trap. Executor faults and instruction-limit exhaustion remain host/executor failures rather than valid guest gas outcomes.

No dedicated RV64 gas register is proposed. Whether a hot cached value is worth reserving a register is an optimization question requiring an explicit ABI/register-convention amendment and measurement. `s0` and `s1` remain reserved for stack state. Existing `a0`–`a2` arithmetic interfaces and scratch requirements must be respected.

## 9. Conformance and Benchmarking

### Correctness suite proposal

Extend the existing `rc_revm_conformance` approach to compare full observable outcomes under REVM Cancun `43.0.3`: terminal category, exception category where stable, full live stack when the reference exposes a comparable state, gas limit, gross gas used/remaining, refund fields when applicable, and relevant output/state. The current harness compares stack outcomes but does not compare gas.

For every compiled opcode, test gas limit below cost, exactly cost, and above cost; one-op and multi-op programs; STOP and implicit fallthrough; OOG position in multi-op sequences; stack underflow/overflow at gas boundaries; exact exception classification and gas consumption; and no partial stack mutation on failed charges. For `GAS`, assert observed value at the opcode point, not only final aggregate gas. Track Cancun-specific tests for each newly introduced opcode family and dynamic rule.

REVM public transaction execution may include intrinsic gas and transaction-layer normalization. Tests must explicitly define whether Ferreth input gas is an execution-frame budget or a transaction gas limit, and adapt/reference the REVM frame accordingly. Do not compare a Ferreth execution budget directly to a transaction gas limit without subtracting intrinsic charges and matching environment/state.

### Metrics

Report EVM accounting separately: execution gas limit, gross execution gas used, remaining gas, refund counter, and applied refund (when relevant). Report RV64 efficiency separately: dynamically executed RV64 instructions under the documented executor semantics, generated code bytes/words, runtime-helper instruction cost, and per-opcode instrumentation overhead. Existing `opcode_core`, `opcode_guest`, and `program_guest` metrics remain RV64 metrics; they cannot stand in for gas.

The present benchmark infrastructure has deterministic workload generation and executor instruction counts (`benchmark.rs`, `execution.rs`, `docs/runbooks/benchmark.md`). It can provide a starting point for comparing pre-/post-metering RV64 instruction counts and code sizes if the report schema adds separate fields. It does not currently collect generated whole-program code-size or gas metrics, and no overhead results are claimed here. Record baseline and metered runs under identical inputs/seeds and avoid folding EVM gas into RV64 totals.

## 10. Future Extension Points

- **Arithmetic/stack:** common static charge hook before mutation; current operations have fixed costs, stack exceptions remain separately classified.
- **Memory:** gas state must support memory high-water mark and Cancun memory expansion calculation; current guest map has no EVM dynamic memory region contract.
- **KECCAK256:** combine base and per-word charge plus memory expansion before hashing effects.
- **Storage/environment:** execution context needs transaction-scoped warm/cold sets, original/current values and fork-aware prices; these are not represented today.
- **CALL/create:** frame-local gas budgets, forwarding/stipend rules, child result and gas return, and state rollback require a nested-frame contract.
- **REVERT/exception:** distinguish successful halt, revert (with its specific gas behavior and output), OOG and other exceptional halts; current generic guest status is insufficient for complete semantics.
- **Control flow:** gas accounting follows actually executed paths; linker labels and exceptional trampolines must converge on shared metering/termination conventions.

These are extension requirements, not implementation authorization or a claim that the corresponding opcodes are supported.

## 11. Consequences and Risks

- Guest-owned accounting preserves architecture and makes the actual executed path authoritative, but adds RV64 instructions and state traffic.
- Memory-backed state avoids inventing a register reservation and makes results inspectable, but a new guest-state/result ABI is likely required and must be reviewed against ADR-001.
- Per-opcode checks provide the simplest correctness baseline and precise failure point; basic-block aggregation may be considered later only with equivalence evidence.
- Dynamic gas makes a scalar counter insufficient for the mature EVM, but a general state record/interface can host future frame, memory and access metadata without implementing those systems now.
- Cancun/REVM comparison must control transaction-vs-frame gas semantics and state inputs; otherwise apparent parity can be misleading.
- Exception codes remain provisional in ADR-001; this proposal needs stable gas/exception classification before claiming exact error conformance.

## 12. Open Questions — Requires Maintainer Review

1. **Gas state ABI location and versioning — OPEN — requires maintainer review.** Should gas fields extend the 24-byte `GuestMetadata`, live in a separate record in the metadata region, or be carried through a versioned execution-state structure? This matters for ADR-001 compatibility and publication atomicity. Evidence: `rv64/guest.rs` record and ADR-001 §5/§8; no gas fields exist. Alternatives: append/version record; separate record; another guest memory location. Extending the record is simple but changes ABI; separate record preserves existing offsets but adds pointer/ownership questions; new location risks another implicit convention. No choice is justified yet.
2. **Initial gas budget API — OPEN — requires maintainer review.** Is the supplied limit a top-level EVM message-call execution budget, a transaction gas limit after intrinsic deduction, or both through separate APIs? Matters because conformance and reported remaining gas differ. Evidence: `execute_guest_evm_bytecode` accepts bytecode and initial stack only; no transaction context. Alternatives: frame budget only initially; transaction API with intrinsic accounting; separate explicit APIs. Frame budget aligns with current scope but must be labeled clearly; transaction semantics increase scope.
3. **Exception taxonomy and precedence — OPEN — requires maintainer review.** Which stable guest result codes distinguish OOG, stack underflow/overflow, REVERT, and future exceptional halts, and in what exact order do gas checks and stack validations occur? Matters for Cancun result and gas correctness. Evidence: provisional exception codes in `orchestrator.rs`; generic guest exception publication and ADR-001 open item. Alternatives: stable typed codes in ABI; opaque code plus host diagnostics; sequentially define per-op semantics from Cancun/REVM. The latter is necessary evidence before a contract can be approved.
4. **Remaining vs consumed gas on exceptional termination — OPEN — requires maintainer review.** Should result expose the frame's remaining gas (zero on exceptional halt), gross gas used, or both? Matters for `GAS`, diagnostics, and REVM comparison. Evidence: current `GuestResult` has no gas data; Cancun exceptional-halt rules consume remaining frame gas. Alternatives: store remaining and derive used; store both and validate; retain only used. Storing both improves auditability but adds redundancy/invariants.
5. **Gas value width and overflow policy — OPEN — requires maintainer review.** Use `u64`, larger integer, or bounded value? Matters for arithmetic and ABI portability. Cancun EVM gas quantities fit practical transaction bounds but the Ferreth API's allowed input range is unspecified. Alternatives: checked `u64`; `U256`; protocol-bounded type. Do not silently wrap.
6. **Static check placement — OPEN — requires maintainer review.** Where should common meter code be emitted and how should it interact with each opcode's validation, helpers, and exceptional trampoline? Matters to avoid duplicated semantics and register clobbers. Evidence: per-op generators and linker in `compiler.rs`, no common opcode prologue. Alternatives: inline generated sequence; shared guest subroutine; linker-inserted charge block. Inline costs code size, a call adds overhead/ABI constraints, linker insertion needs robust metadata.
7. **Dynamic state ownership — OPEN — requires maintainer review.** Which future layer owns memory high-water mark, warm/cold access sets, refund counter, original storage values and child frame state? Matters because future gas formulas depend on transaction/frame context. Evidence: none of these fields exist; ADR-001 explicitly leaves memory/storage/gas for later. Alternatives: one extensible guest context; specialized host-provided tables with guest calls; hybrid. Host callbacks during execution risk violating guest autonomy; fully guest memory requires representation and helper APIs.
8. **Cancun normative corpus/version — OPEN — requires maintainer review.** Which pinned execution-specifications commit/release and Cancun test-vector corpus should be recorded alongside REVM `43.0.3`? Matters for reproducible fork semantics; current REVM version is pinned but an execution-spec source revision is not recorded. Alternatives: pin repository commit; pin a release/tag; treat EIPs plus REVM as sources. Recommend pin before implementation, but exact revision requires maintainer selection.
9. **Benchmark report schema — OPEN — requires maintainer review.** Add gas fields to existing benchmark JSON or create a separate gas-conformance report? Matters for consumers and keeping RV64 metrics distinct. Evidence: existing benchmark JSON schema and runbook describe only RV64 counts. Alternatives: separate namespaces/schema version; separate report; versioned extension. A separate or versioned namespace is advisable but not required by current architecture.

## 13. Compatibility with ADR-001

| ADR-001 decision | Relation | Review note |
|---|---|---|
| Guest-owned stack in memory; `s0`/`s1` reserved | Compatible | Proposed gas state does not repurpose stack registers. |
| One linked guest program, no per-opcode Rust callback | Compatible with guest metering; conflict with host dispatch | Option A would alter this boundary if implemented as dispatch. |
| 24-byte metadata/result record | Potential conflict | Gas publication requires extension or companion record; explicit ADR-001 ABI review is required. |
| Status/result published before PC sentinel | Compatible with required gas publication | Requires all gas fields to publish before terminal status validity; exact ordering is open. |
| Generic/provisional guest exception code | Potential conflict | Exact OOG classification requires stable codes; ADR-001 already lists this as open. |
| Backend-neutral `Rv64Executor` | Compatible | Host initializes/reads gas state through existing memory methods; no emulator-specific gas semantics. |
| `opcode_core`/guest/program instruction metrics | Compatible | Retain as RV64 counts and add separately named EVM gas metrics. |
| Future gas listed as open work | Compatible | ADR-002 specializes that open area; it does not alter ADR-001 text. |

## 14. Adversarial Review of This Proposal

- **Accepted ADR contradiction:** No accepted decision reserves a gas register or defines gas ABI. This proposal avoids reserving a register and labels the likely state/result ABI change as open. Host opcode dispatch is rejected as conflicting with the one-run guest model.
- **Hidden Rust dependency:** Metering is guest-authoritative; Rust supplies initial state and reads final state only. A future design that requires per-opcode callbacks would need explicit architectural review.
- **Termination protocol:** OOG is proposed to publish through the existing guest exception path and sentinel, not a native trap. It requires new result classification.
- **State integrity:** Charge-before-effect and precondition ordering are specified as invariants, while the precise order for competing exceptional conditions remains open pending source verification.
- **Exceptional gas:** Proposal states Cancun OOG consumes frame remaining gas but requires exact REVM/Cancun conformance tests; result field decision is open.
- **Gas reporting:** Separates gas limit, gross consumed, remaining, and refunds from RV64 metrics; exact ABI representation is open.
- **Dynamic gas/nesting:** Interfaces are identified but frame context, memory expansion, warmth, refund and forwarding models are not prematurely implemented.
- **Error precedence:** Explicitly open, not asserted as settled.
- **Fork verification:** Cancun is pinned conceptually and REVM `43.0.3`/`SpecId::CANCUN` is grounded in manifests/source; a pinned execution-spec revision remains open.
- **REVM conformance:** Existing harness provides a grounded path but must be extended to compare gas and exceptional outputs at frame-equivalent inputs.
- **Unresolved decisions:** ABI, input scope, taxonomy, reporting fields, width, meter placement, dynamic state, normative corpus, and report schema remain visibly open.

## 15. References

### Repository

- [ADR-001: EVM Guest State and RV64 Runtime ABI](001-evm-guest-state-abi.md), especially §§3–5, 8–13.
- `crates/ferreth-compiler/src/compiler.rs`: `compile_evm_bytecode`, `compile_supported_program`, linker, termination publishers.
- `crates/ferreth-compiler/src/rv64/guest.rs`: `GuestMetadata`, `GuestResult`, `read_guest_result`, stack state.
- `crates/ferreth-compiler/src/rv64/memory.rs`: memory map, stack bounds, code/exit layout.
- `crates/ferreth-compiler/src/rv64/executor/mod.rs` and `rv64/executor/rvemu.rs`: host/guest execution boundary and metrics.
- `crates/ferreth-compiler/src/orchestrator.rs`: guest setup, execution, result collection.
- `crates/ferreth-compiler/src/execution.rs::run_evm_bytecode`: REVM Cancun reference.
- `crates/ferreth-compiler/tests/rc_revm_conformance.rs`: compiled guest differential harness.
- `crates/ferreth-compiler/src/benchmark.rs`, `docs/runbooks/benchmark.md`: RV64 metric scopes.
- `crates/ferreth-compiler/Cargo.toml`: `revm 43.0.3`, `rvemu 0.0.11`.

### Cancun specifications and EIPs

- Ethereum execution-specifications, Cancun fork: <https://ethereum.github.io/execution-specs/src/ethereum/forks/cancun/>.
- EIP-2200, net SSTORE metering: <https://eips.ethereum.org/EIPS/eip-2200>.
- EIP-2929, warm/cold state access: <https://eips.ethereum.org/EIPS/eip-2929>.
- EIP-3529, reduced refunds and one-fifth cap: <https://eips.ethereum.org/EIPS/eip-3529>.
- Cancun memory expansion, opcode gas and exception behavior must be traced to the corresponding Cancun execution-spec source and pinned REVM 43.0.3 source during implementation review.

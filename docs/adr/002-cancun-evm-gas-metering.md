# ADR-002: Cancun EVM Gas Metering

- **Status** — Accepted. Implementation pending.
- **Maintainer approval date** — 2026-11-23.
- **Originally proposed** — 2026-11-17. Reconciled against v0.1.0 and the subsequent reference experiment.
- **Decision** — Adopt guest-authoritative, fixed-cost Cancun frame metering for the existing six opcodes and GAS.

## Context and evidence

[Accepted ADR-001](001-evm-guest-state-abi.md) establishes a single autonomous guest image, ascending four-limb little-endian U256 words, 1,024 stack slots, base s0 and next-free s1, a 24-byte terminal prefix and pre-fetch sentinel termination. The original proposal described patchable bodies. At v0.1.0 production uses symbolic fragments, one assembly pass and independent final-image checks. The original frame-oracle prerequisite has experimental evidence. See the [readiness assessment](../research/adr-002-readiness.md), [37-vector corpus](../research/adr-002-reference-corpus.md) and [incremental implementation plan](../research/adr-002-tdd.md). This decision extends ADR-001.

## Execution semantics

The caller supplies a u64 frame budget. For each executed opcode, compare remaining gas with its cost as unsigned. On insufficiency enter OOG before any stack effect. Otherwise debit exactly once, then validate the stack before operand writes, pointer changes or arithmetic calls. Equality permits execution. Each successor sees the previous debit. On any guest exception preserve the failing operation's input stack and earlier successful effects, and settle remaining gas to zero.

| Operation | Cost | Operation after charge |
|---|---:|---|
| STOP | 0 | Complete normally, including with zero remaining. Ignore trailing bytecode. Empty and implicit EOF also complete without charge. |
| PUSH1 | 3 | Check capacity and push the immediate, zero-padding missing data. |
| POP | 2 | Require one word and retire it. |
| DUP1 | 3 | Require one word and space, then copy the top word. |
| ADD | 3 | Require two words, replace them with their sum modulo 2^256. |
| SUB | 3 | Require two words, replace them with top minus next modulo 2^256. |
| GAS | 2 | Check capacity and push post-charge remaining gas in the low limb with three zero upper limbs. |

Portable tests require terminal class, exact settled remaining and gross used, and exact successful stack. Charge-first diagnostic precedence is a Ferreth contract. EELS pops before charging POP, ADD and SUB. REVM charges before dispatch. Terminal PCs, raw exception names and exceptional stacks remain diagnostic across engines. V22's settled REVM value is source-derived. See [observations and normalization](../research/adr-002-reference-corpus.md).

## Guest state and publication

Extend the existing allocation at 0x80100000 in place. All six fields are little-endian aligned u64 words. Total encoded length is 48 bytes. The first three offsets and status values are ADR-001 contracts. ADR-002 defines the gas fields and OOG code.

| Offset | Field | Initialization and ownership | Terminal meaning and validation |
|---|---|---|---|
| 0 | status | Host writes RUNNING (0). Guest writes terminal status last. | NORMAL (1) or GUEST_EXCEPTION (2), accepted after executor observes EXIT_PC. Reject RUNNING and unknown values. |
| 8 | exception | Host initializes zero. Selected guest publisher writes its code. | Normal requires 0. Exception requires overflow 1, underflow 2 or OOG 3. |
| 16 | final s1 | Host uses existing RUNNING initialization. Publisher writes live s1. | Validate range, alignment, depth, live pointer and base as today. |
| 24 | input limit | Host writes caller budget once. Guest reads only. | Must equal host's original budget. |
| 32 | live remaining | Host writes input limit. Generated guest debits on successful checks. Exceptional publisher sets zero. | At terminal, must equal published remaining and be within input limit. |
| 40 | published remaining | Host initializes zero as an incomplete result. Publisher explicitly stores live remaining for success or zero for exception. | Require equality with live gas and zero on exception. Status-last, executor boundary and verified store establish validity even when zero is legitimate. |

Publish final s1, code and gas before terminal status, then jump to the existing sentinel. The host reads the record after successful executor completion and derives gross used by checked subtraction of terminal remaining from the original input. Return validated limit, remaining and gross used through GuestResult accessors. Malformed metadata, emulator faults and instruction-limit errors follow host error paths. No valid gas value doubles as an unpublished marker.

## Execution and compiler interfaces

The canonical public bytecode entry point is `execute_guest_evm_bytecode(bytecode: &[u8], gas_limit: u64)`, returning `Result<OrchestratorResult, OrchestratorError>`. It replaces the historical one-argument signature. The CLI resolves an omitted budget to `u64::MAX` before calling it and passes explicit budgets through, including zero. Isolated guest APIs delegate to the same internal stack-and-budget execution implementation. All callers share opcode admission, compilation, generated bodies, execution and publication. Positive-cost instructions debit it normally. The caller and the budget cannot select a metering switch or an alternate route. compile_evm_bytecode takes bytecode alone. On the experimental implementation branch, each increment updates this single implementation in place. Only the completed, validated seven-opcode milestone carries the public Cancun gas accounting contract. Exact guest instruction counts, image bytes and maximum source lengths can change with instrumentation. Standalone arithmetic-core measurement keeps its current scope.

Use a shared Rust emission routine to insert inline RV64 checks at each positive-cost symbolic fragment entry. Load, compare unsigned, and subtract only on sufficiency. Derive metadata addresses from s0 as publishers already do. Scratch t0–t2 may be clobbered before operand setup. Preserve s0, native sp and ra, and change s1 only for a successful stack operation. Keep no live gas in a caller-saved register across a leaf call. Share the ADD/SUB wrapper and preserve the arithmetic leaf bytes. Zero-cost STOP needs only its existing normal transfer.

Declare a fragment-local conditional transfer and symbolic JAL to a shared OOG publisher, with occurrence-specific local labels. Register OOG in publisher inventory, boundaries and instruction accounting. Update word indices for existing transfers including arithmetic call/return. Extend independent fixed-layout verification for metered guards, expected successors, leaves, exact publisher stores, status-last and sentinel JALR. Include all reachable role combinations. Retain declaration verification and reject undeclared or misdirected transfers. Test long layouts with OOG beyond conditional-branch reach. The code reservation, assembler limits and host step bound still apply.

## Alternatives, limits and approval

Inline memory-backed charging avoids a new register reservation, guest helper ABI or per-opcode host callbacks. Basic-block aggregation and persistent register caching are deferred pending evidence. Gross used is derived, with no separate consumed counter. Future reversible state would have its own rollback rules. Transaction intrinsic gas, dynamic pricing, memory expansion, storage, refunds, external calls, other forks and backend integrations are excluded. RV64 instruction counts measure execution work independently of frame gas.

The maintainer approved the incremental TDD plan alongside this decision. Implementation and milestone validation are pending.

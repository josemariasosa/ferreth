# ADR-001: EVM Guest State and RV64 Runtime ABI

- **Status:** Accepted
- **Implementation:** Implemented for the six-opcode compiler subset
- **Date:** 2026-11-17

## Implementation Summary

ADR-001 is implemented for `STOP`, `PUSH1`, `POP`, `DUP1`, `ADD`, and `SUB`. Ferreth provides a defined RV64 guest memory layout, an ascending U256 stack, a result-publication ABI, PC-sentinel termination, reusable arithmetic cores, and a backend-neutral executor interface.

Both the Rust Orchestrator (RO) and Rust Compiler (RC) execute generated RV64 instructions through the shared guest ABI. Recorded evidence includes unit tests, integration tests, deterministic whole-program RC-to-REVM differential tests, and instruction-count benchmarks. [Guest linking architecture and evidence](../research/ferreth-guest-linking.md)

**Remaining work:** Broader EVM opcode coverage, stable exception-code definitions, and optional alternative execution backends. Native-stack execution and full EVM compatibility remain outside the initial ADR scope.

| Component | Status | Evidence / Remaining work |
|---|---|---|
| RV64 guest memory map | Implemented | `rv64/memory.rs` defines and validates the accepted code, metadata, EVM-stack, unassigned, and native-stack regions. |
| Guest stack layout and U256 representation | Implemented | `rv64/guest.rs` encodes four little-endian `u64` limbs per 32-byte word and validates the 1024-word ascending stack. |
| Guest state initialization (`s0`, `s1`) | Implemented | `initialize_guest_state` initializes both registers to the EVM stack base and initializes metadata to `RUNNING`. |
| Guest status and result-publication ABI | Implemented | The 24-byte metadata record is encoded, validated, and published by generated normal and exceptional termination paths. |
| Normal completion and guest exceptions | Implemented | `STOP` and EOF lead to the normal publisher. Stack overflow and underflow lead to exceptional publishers with provisional codes. |
| Backend-neutral `Rv64Executor` interface | Implemented | `rv64/executor/mod.rs` exposes load, memory/register access, bounded execution, completion, and metrics without rvemu types. |
| rvemu execution backend | Implemented | `RvemuExecutor` provides the initial rvemu 0.0.11 adapter and pre-fetch exit-sentinel accounting. |
| Shared RV64 arithmetic cores | Implemented | `rv64/opcodes/add.rs` and `rv64/opcodes/sub.rs` wrappers call the reusable pointer-based cores used by both RO and RC. |
| Rust Orchestrator (RO) execution path | Implemented | `orchestrator.rs` runs isolated PUSH1, POP, DUP1, ADD, and SUB guest programs through the shared ABI. |
| Rust Compiler (RC) whole-program execution path | Implemented | `compiler::compile_evm_bytecode` produces one verified RV64 executable through the symbolic composer. `orchestrator::execute_guest_evm_bytecode` runs it and validates the published guest result. |
| Supported EVM opcodes | Implemented for the bounded subset | RC supports `STOP`, `PUSH1`, `POP`, `DUP1`, `ADD`, and `SUB`. Decoding additionally recognizes `PUSH2`–`PUSH32`, which RC rejects. |
| REVM differential validation | Covered for the supported subset | Directed and seeded randomized whole-program RC tests compare generated RV64 execution with REVM 43.0.3 configured for `CANCUN`; broader opcode coverage remains future work. |
| Benchmark scopes (`opcode_core`, `opcode_guest`, `program_guest`) | Implemented | `benchmark.rs` produces the arithmetic baseline and deterministic guest/program measurements with split body and termination counts. |
| Native RV64 execution or alternative executor backends | Not in initial scope | The native stack is reserved but unused; rvemu is the only backend. ADR-001 explicitly leaves Unicorn/other backend work optional. |

## 1. Context

The following context describes the arithmetic prototype at the time of the
ADR decision. See the implementation summary above for the current guest route.

`ferrum-eth-translator` currently supports a narrow experiment: it decodes
`PUSH1`–`PUSH32`, `ADD`, and `SUB`; generates RV64 arithmetic routines for
`ADD` and `SUB`; and checks arithmetic against `revm`. Arithmetic values are
represented by four `u64` limbs, least-significant limb first. The generated
arithmetic functions receive operand and destination pointers in `a0`, `a1`,
and `a2`. The current `ProgramRunner` executes decoded instructions while
managing its EVM stack in Rust; `PUSH` has no corresponding RV64 guest
operation.

The existing emulator harness creates `rvemu::Emulator`, loads generated code
with `initialize_dram`, initializes operand memory and `a0`–`a2`, sets `ra`
(`x1`) to a return sentinel, and sets the PC to `DRAM_BASE`. It executes
instructions until control reaches the sentinel, counting successfully
executed 32-bit instructions and excluding the sentinel. The current harness
does not establish an EVM guest memory layout, initialize a native `sp`
convention, or execute a complete EVM program in RV64.

The intended architecture separates EVM definitions and execution-state
contracts (`evm/`), RV64 generation and guest primitives (`rv64/`), isolated
opcode execution/testing/benchmarking (`orchestrator/`, RO), and complete
program translation (`compiler/`, RC). RO and RC are expected to share the
RV64 opcode implementations and a compatible guest-state representation.
These are architectural goals, not existing repository components.

This ADR records the accepted initial shared guest ABI and the boundary between
generated code and the host executor. ADR acceptance authorizes implementation
according to this contract; it does not claim that the complete guest runtime
is implemented or validated. Runtime conformance is a later milestone. This
ADR does not specify a complete EVM implementation, production error taxonomy,
gas accounting, persistent storage, or dynamic EVM memory.

## 2. Decision Drivers

- Preserve EVM 256-bit word semantics while using native RV64 operations.
- Let isolated opcode runs and compiled programs share the same operand-stack
  representation and opcode implementations.
- Keep EVM stack semantics separate from the native RISC-V call stack.
- Make final guest state observable to Rust, including on exceptional exit.
- Bound accesses to the physical memory exposed by the selected executor.
- Preserve a comparable dynamic RV64 instruction-count measurement as execution
  expands from arithmetic routines to guest operations and full programs.
- Keep the guest ABI independent of executor-specific memory, traps, and
  execution-control APIs.

## 3. Decision

Adopt a shared guest ABI with a maximum of 1024 EVM words, each 32 bytes and
stored as four consecutive 64-bit limbs in least-significant-limb-first order.
The 1024-word bound is the EVM architectural stack limit, not a configurable
project capacity. The stack grows toward increasing guest addresses and is
independent of the native RISC-V `sp`.

Reserve `s0` (`x8`) for the EVM stack base and `s1` (`x9`) for the next-free
stack address. Derive depth from `(s1 - s0) / 32`; no dedicated depth register
is required initially. Keep these hot pointers in registers during execution
and maintain a guest-memory metadata/result record for initialization and
host-visible completion. Publish the final status, exception code when
applicable, and `s1` before any deliberate normal or guest-exception exit.
The host derives and validates final depth and reads the live stack words from
guest memory. The ABI does not require synchronizing the complete state after
every opcode.

RO should call the same reusable RV64 opcode implementations used by RC, with
a thin adapter for isolated test inputs. Preserve the existing pointer-based
ADD/SUB arithmetic cores as leaf implementations behind stack-aware guest
wrappers; do not duplicate their arithmetic logic. Keep the existing
`opcode_core` benchmark comparable with historical results.

The guest result contract is independent of how an executor detects exit.
For initial rvemu execution, reserve PC `0x800FFFFC` as an exit sentinel,
checked before the next instruction is fetched or executed. The sentinel is
the final four-byte slot in the code reservation and is reserved exclusively
for deliberate guest termination; generated executable code must not occupy
it, and normal guest control flow must never enter it accidentally. `STOP`
means successful EVM execution termination: publish the final result record,
then deliberately jump to the exit PC. It is not a native RISC-V `ret`,
`EBREAK`, or `ECALL`. Guest exceptions, emulator failures, and host
instruction-limit exhaustion remain distinct outcomes.

The intended code boundaries are `evm/` for EVM semantics, decoding, and state
definitions; `rv64/` for code generation, opcode implementations, guest
runtime, and executor infrastructure; `orchestrator/` (RO) for isolated
opcode execution, differential validation, and benchmarking; and `compiler/`
(RC) for whole-program EVM-to-RV64 compilation and execution. This is the
shared architecture, not a claim that those components already exist.

## 4. Guest State and Memory Model

### Operand stack representation

Each EVM word occupies 32 contiguous bytes and is represented as four
little-endian 64-bit limbs:

| Offset from word address | Content |
| --- | --- |
| `+0` | limb 0, least significant 64 bits |
| `+8` | limb 1 |
| `+16` | limb 2 |
| `+24` | limb 3, most significant 64 bits |

The least-significant limb is at the lowest address, and bytes within each
limb use little-endian order. This matches the explicit multi-byte load/store
behavior in rvemu 0.0.11's DRAM implementation. The current arithmetic core
also processes limbs from offset 0 upward.

For the architectural maximum of 1024 words, the stack occupies 32 KiB. Given
base `s0` and depth `d`, its next-free address is `s1 = s0 + 32*d`. The top
item, when present, begins at `s1 - 32`. Depth zero means `s1 == s0`; full
capacity means `s1 == s0 + 32768`.

### Initial rvemu physical layout

The initial rvemu layout uses these **half-open** intervals:

| Region | Address interval | Initial role |
| --- | --- | --- |
| Generated RV64 code and exit boundary | `[0x80000000, 0x80100000)` | 1-MiB code reservation. Entry PC is `0x80000000`; the final four-byte slot `[0x800FFFFC, 0x80100000)` is reserved as the exit PC and is not executable code. |
| Guest metadata/result | `[0x80100000, 0x80101000)` | 4-KiB metadata allocation; the 24-byte result record begins at `0x80100000`. |
| EVM operand stack | `[0x80101000, 0x80109000)` | Exactly 32 KiB for 1024 words; use its base as `s0` and its exclusive upper bound as `stack_limit`. |
| Unassigned space | `[0x80109000, 0xBFFF0000)` | Not available to guest state without a later layout decision. |
| Native RV64 stack reservation | `[0xBFFF0000, 0xC0000000)` | 64-KiB reservation, independent of the EVM operand stack; use only if runtime code needs it. |

The 1-MiB code reservation, 4-KiB metadata allocation, and 64-KiB native
stack reservation are initial implementation choices, not RISC-V or EVM
requirements. With entry PC `0x80000000`, the maximum generated code size is
`0xFFFFC` bytes: code starts at the entry PC, has a length divisible by four,
and occupies a half-open interval ending no later than `0x800FFFFC`. Thus the
entry and exit PCs are four-byte aligned, the exit slot is inside the code
reservation but outside generated code, and the executable cannot overlap
metadata. Validate the code size, entry address, and alignment together.
Keep half-open bounds in the guest contract and validate the entire byte
range of each host-mediated access, including its width. Future executor
backends may use different physical addresses while preserving the logical
guest ABI.

rvemu 0.0.11 defines `DRAM_BASE` as `0x80000000` and allocates 1 GiB, giving
the intended DRAM interval `[0x80000000, 0xC0000000)`. Its bus dispatch uses an
inclusive range through `DRAM_END`, however, while the DRAM byte vector has
indices only through `DRAM_SIZE - 1`. An access beginning at the nominal
exclusive end, or a multi-byte access crossing a region end, can therefore
index outside the backing vector. Host-side address and access-width checks
are required; do not rely on the bus range as a safe bounds check.

rvemu uses shared DRAM for instruction fetches and data accesses and does not
provide guest-level memory protection between our logical regions. Host-side
address validation protects host-mediated loading and memory access, but does
not prevent arbitrary guest RV64 stores from writing outside their intended
regions. Generated stack primitives must implement the required boundary
checks. The initial runtime is a trusted-code research environment, not a
sandbox for arbitrary untrusted RV64 binaries. `initialize_dram` replaces
bytes at the start of DRAM without clearing the remainder; `cpu.reset()`
resets CPU state but does not clear guest memory. Reinitialization must
explicitly overwrite or clear every region whose previous contents could be
observed. These are source-inspection findings, not substitutes for runtime
tests of the initial layout.

### Stack growth

The ascending stack is an ABI decision. `s0` is the first stack byte, `s1`
points to the next free 32-byte slot, and `stack_limit` is the exclusive upper
bound. The top item begins at `s1 - 32` when nonempty.

## 5. Register Convention and ABI Invariants

The guest runtime convention is:

| Register | Meaning | Compatibility |
| --- | --- | --- |
| `s0` (`x8`) | EVM operand-stack base address | Reserved by the guest ABI. |
| `s1` (`x9`) | Address of the next free EVM stack slot | Reserved by the guest ABI. |
| `sp` (`x2`) | Native RV64 stack pointer, independent of the EVM stack | Not required unless generated code uses the native stack. When used, observe the applicable psABI alignment and remain within its reservation. |
| `a0`–`a2` (`x10`–`x12`) | Existing arithmetic core: operand A, operand B, destination pointers | Preserve the pointer-based core interface; guest wrappers adapt stack slots to it. |
| `t0`–`t5` | Scratch registers used by current ADD/SUB cores | Caller-saved temporaries; do not keep live guest state in them across a core call. |
| `ra` (`x1`) | Standard return address when a routine call uses it | It is not the guest execution exit contract. |

`s0` and `s1` are callee-saved registers under the RISC-V integer calling
psABI. Source inspection of the current generated ADD/SUB cores confirms they
use `a0`–`a2` for pointers, `t0`–`t5` as scratch, do not modify `s0` or `s1`,
and do not use the native stack. These register assignments are compatible
with those cores. The guest ABI's custom register reservation still applies
to every generated RV64 routine and wrapper.

The following invariants define the accepted guest ABI:

1. `s0 <= s1 <= stack_limit`.
2. `s1 - s0` is divisible by 32.
3. Depth is `(s1 - s0) / 32` and is in `0..=1024`.
4. A valid push requires depth below 1024; validate before writing the word or
   advancing `s1`.
5. A valid pop requires depth above zero; validate before retreating `s1`.
   Popped bytes need not be cleared because they are outside the live stack.
6. Binary operations require at least two operands. With the top operand first,
   `a0 = s1 - 32` and `a1 = s1 - 64`. Set `a2 = s1 - 64`, reusing the lower
   operand's slot as the result; after successful execution decrement `s1` by
   32. For `SUB`, the core computes top-of-stack minus the next operand.
7. An explicitly handled guest error must preserve these stack invariants and
   must validate preconditions before modifying live stack state.
8. The EVM operand stack and native `sp` remain separate. The native stack, if
   used, has independent bounds and alignment.

The current limb-by-limb ADD/SUB cores load both input limbs before storing the
corresponding output limb, so exact destination aliasing with either input is
safe for these routines when the destination does not partially overlap the
other input. Do not assume partial destination/input overlap is safe: an
earlier store could overwrite a later input limb. Stack-wrapper integration
and aliasing constraints require targeted tests before being considered
validated.

Deriving depth from `s0` and `s1` avoids an extra mutable count and prevents
register/metadata count drift. It is a good initial choice for a bounded stack;
it costs subtraction and shift/division if depth is frequently needed and
requires reliable access to both pointers. A separate count register is
unnecessary initially. Guest memory should hold a canonical result depth or
final pointer for host retrieval rather than duplicating a live count on every
operation. The host validates both pointers and derives final depth from them.
Revisit this trade-off only if later profiling or ABI requirements justify it.

## 6. Initialization and Termination Contract

### Before execution

Rust, as the host harness, is responsible for:

- Constructing and configuring the emulator, loading generated code and
  required guest regions, and setting the entry PC.
- Enforcing host-side region bounds and initializing the result record before
  execution.
- Setting `s0` and `s1` consistently for an empty stack, with depth zero.
- Initializing all live stack words (none at depth zero); no value may be read
  from an uninitialized slot.
- Initializing `status` to `RUNNING`, with `exception_code` zero and
  `final_s1` initialized consistently with the empty stack.
- Initializing any required runtime context, including native `sp` if generated
  code uses the native stack.
- Enforcing a host-side execution-step limit and retaining enough information
  to distinguish a guest result from failure of the emulator or harness.

The 24-byte result record begins at `0x80100000` and consists of three
little-endian `u64` fields:

| Offset | Field | Type |
| --- | --- | --- |
| `+0x00` | `status` | `u64` |
| `+0x08` | `exception_code` | `u64` |
| `+0x10` | `final_s1` | `u64` |

The numeric status values are:

| Name | Value | Meaning |
| --- | --- | --- |
| `RUNNING` | `0` | Execution is in progress or no result has been published. |
| `NORMAL` | `1` | Guest execution completed normally. |
| `GUEST_EXCEPTION` | `2` | Guest execution completed with a guest-published exception. |

Rust initializes the record before execution. At deliberate guest termination,
the guest publishes fields in this order:

1. Write the final stack pointer to `final_s1`.
2. Write `exception_code` if terminating exceptionally; for normal
   termination it is zero.
3. Write `status` last (`NORMAL` or `GUEST_EXCEPTION`) to signal publication.
4. Transfer control to the executor's exit PC.

The host recognizes termination only at the expected exit boundary, accepts
only `NORMAL` or `GUEST_EXCEPTION` status there, validates the published
stack-pointer invariants, and retrieves live stack contents from guest memory.
`RUNNING` or an unknown status at the exit boundary is invalid. A result is
valid only after deliberate guest termination and successful validation of
the published record; an incomplete record is never a valid EVM result. Guest
exception-code ownership and future encoding belong to the EVM guest runtime
contract; exact numeric codes remain provisional. No atomics or additional
synchronization are required for this single-threaded initial runtime.

Generated code owns valid guest stack transitions during execution. An
emulator failure can leave partially changed registers or memory and does not
produce a valid EVM result; no rollback is guaranteed. No full-state memory
synchronization is required after every opcode.

The pre-ADR arithmetic harness demonstrated code loading with `initialize_dram`,
initialization of data using `cpu.bus.write`, direct integer-register setup,
and `initialize_pc`. It constructed the emulator with `Emulator::new`, which
has DTB/DTS side effects relative to the working directory. The prototype did
not demonstrate custom region setup, stack initialization, or a complete guest
result protocol. In rvemu 0.0.11 the host can inspect CPU
registers and bus memory directly, while `cpu.execute()` returns a typed
exception on instruction failure. These interfaces support the intended
minimal executor boundary but still require runtime tests for result
publication and memory layout.

### `STOP` and host return

Conceptually, EVM `STOP` marks normal guest completion, preserves the final
observable execution state, and transfers control to Rust. It does not return
a single word: the stack may contain zero, one, or multiple live values.

The guest result contract must expose at least:

| Result field | Meaning |
| --- | --- |
| Execution status | Successful EVM termination or a guest-published exceptional termination. |
| Guest exception code | `u64`; zero for normal termination, guest-published for exceptions. Exact numeric assignments and encoding remain open. |
| Final next-free stack pointer | The published `s1`; host validates it against `s0` and `stack_limit`. |
| Final stack depth | Derived by the host as `(s1 - s0) / 32` after pointer validation. |
| Final live stack contents | All live 32-byte words in ascending address order; the top is the last word. |

The initial rvemu execution protocol is:

1. Guest publishes `final_s1`, then `exception_code` when applicable (zero for
   normal termination), and writes `status` last.
2. Guest deliberately jumps to `exit_pc = 0x800FFFFC`.
3. The host checks `pc == exit_pc` before fetching or executing the next
   instruction.
4. The host verifies the expected exit boundary, validates the published
   status and stack pointers, then retrieves the final live stack contents.

Rust recovers the result from the published record and executor interface; it
does not need to replay or interpret individual EVM opcodes.

The final successfully executed jump is included in dynamic instruction
counts. The exit sentinel itself is never executed and is not counted. This
is an rvemu executor-specific mechanism, not a universal guest ABI requirement.
Another executor may use different control mechanisms while observing the
same result contract.

In rvemu 0.0.11, `cpu.execute()` returns typed exceptions; executing `EBREAK`
returns a breakpoint exception and `ECALL` returns an environment-call
exception. Neither is the selected successful guest exit. The current
arithmetic harness already uses a PC sentinel and checks it before executing
the appended `EBREAK`, so its existing pattern supports this initial protocol.
EVM `STOP` denotes successful EVM termination; it is not equivalent to native
`ret`, `EBREAK`, or `ECALL`.

## 7. Error Handling

This ADR does not define a complete production EVM error model. The initial
ownership boundary is:

| Condition | Owner | Contract |
| --- | --- | --- |
| Stack underflow/overflow | Guest runtime execution | Validate before modifying live stack state; publish an exceptional status while preserving the stack invariants. Exact EVM error-code assignments are open. |
| Invalid/unsupported opcode in input | Compilation/decoding | Reject before guest execution with byte offset and opcode where available. Do not silently substitute a guest operation. |
| Unsupported or malformed generated RV64 | Compilation / host setup | Report generation or assembly/validation failure, not an EVM exception. |
| Normal termination (`STOP`) | Guest runtime execution | Normal status and preserved final state. |
| Emulator fetch, memory, or execution failure | Host executor / emulator | Executor failure, distinct from a guest-published exception. Do not treat potentially incomplete guest state as an EVM result or promise rollback of a partially executed RV64 operation. |
| Execution-step limit exceeded | Host executor policy | Host-limited/incomplete execution, not normal guest termination and not automatically an EVM exception. |

Where the emulator reports a trap caused by guest-generated code, the host
must not relabel it as normal termination without an explicit guest protocol.
For anticipated guest errors, validate required preconditions before mutating
live stack values or pointers. Unexpected emulator failures are not
transactional: partial register or memory updates may remain, and such state
must not be treated as a valid published guest result.

## 8. Benchmark Compatibility

Retain three measurement scopes and report dynamic RV64 instruction counts,
not static code size, elapsed time, hardware cycles, or EVM gas. Current rvemu
counts successfully executed instructions: the harness fetches the instruction
at the PC, calls `cpu.execute()`, then increments the metric only on success.
The PC sentinel is detected before fetch/execute; an executed guest jump to it
is counted, but the sentinel is not. Preserve the existing arithmetic
`opcode_core` benchmark and its results.

Report distinct measurement scopes:

| Metric | Measurement boundary |
| --- | --- |
| `opcode_core` | Dynamic instructions in the isolated pointer-based arithmetic core. Exclude host setup and the host return sentinel. Preserve historical ADD/SUB comparability. |
| `opcode_guest` | Dynamic instructions for one guest opcode, including guest-side stack checks, loads/stores, pointer updates, status/error checks, and opcode body. Exclude executor initialization and host-only result extraction. |
| `program_guest` | Dynamic instructions for a full generated program, including guest initialization/dispatch and any guest-executed termination code. Exclude executor construction, code/data loading, host initialization, and host result retrieval. |

Counts are executor-specific measurements. rvemu counts successfully
executed instructions; Unicorn code hooks fire before execution and may observe
instructions that then fault. Record emulator identity and exact counting
semantics with each benchmark report. Do not automatically treat counts from
different engines as equivalent. Keep host policy such as instruction limits
visible where it may affect completion, and exclude host-only fetch/decode work
from guest instruction counts.

The current `InstructionMetrics` records the fetched instruction only after
`cpu.execute()` succeeds and reports aggregates across cases, so rvemu's
current count represents successfully executed instructions.
In `Rv64Runner` the sentinel is detected before execution. The benchmark also
reports static routine instruction words separately; that value must not be
presented as the dynamic count.

## 9. Consequences and Trade-offs

- The 1024-word bound is the EVM architectural maximum. Its 32-KiB stack region
  is reserved even when little is live.
- Increasing-address growth makes pointer arithmetic and depth derivation
  simple; it is now an ABI commitment for generated code.
- The hybrid register-plus-memory contract keeps hot state available to guest
  code while offering a stable host result boundary. It adds synchronization
  obligations: final status, exception code when applicable, and stack pointer
  must be published before deliberate termination.
- Sharing the guest contract lets RO test the same opcode behavior RC uses.
  Isolated core measurements remain useful but do not measure stack handling
  or whole-program overhead.
- Existing `ADD`/`SUB` assembly can be retained as arithmetic cores, but
  adapting stack slots to their pointer parameters requires careful operand
  order, exact-aliasing discipline, targeted integration tests, and adherence
  to callee-saved register rules.
- The accepted initial physical map is specific to the rvemu executor. The
  logical guest ABI is independent of those addresses and of executor-specific
  exit mechanisms.
- rvemu 0.0.11 uses shared DRAM for code and data and provides no guest-level
  protection between logical regions. Host-side address validation protects
  host-mediated loading and access, but cannot prevent arbitrary guest RV64
  stores from leaving their intended regions; generated stack primitives
  must enforce stack bounds. The initial runtime is trusted-code research
  infrastructure, not a sandbox for arbitrary untrusted binaries. Unexpected
  emulator failures may leave partially modified state and do not guarantee
  rollback.
- The executor boundary allows future replacement without making EVM semantics
  or the RV64 compiler depend on rvemu-specific Rust types.
- Full gas accounting, persistent storage, dynamic EVM memory, and complete
  EVM exception semantics remain outside this decision.
- The existing Rust-managed `ProgramRunner` behavior remains distinct from
  the guest runtime until a later implementation adopts this ADR.

## 10. Alternatives Considered

### All stack metadata in registers

Fast and avoids memory loads, but makes host-side state retrieval dependent on
register inspection and risks losing the canonical final state at a trap or
context boundary. Rejected as the sole representation; registers remain the
accepted live representation.

### All stack metadata in guest memory

Provides a straightforward host-visible record and avoids reserving registers,
but requires loads/stores for every stack operation and does not provide a
simple shared fast-path convention. Not selected as the sole live
representation; metadata memory remains the canonical initialization/result
state.

### Dedicated depth register

Makes depth reads direct but duplicates information encoded by `s0` and `s1`,
creating a consistency invariant on every stack operation. Defer unless
measurement or later execution requirements justify it.

### Descending stack

Conventional native stacks often grow downward, but the EVM stack is a
separate logical structure. An ascending stack offers simple base/limit checks
and a next-free pointer; neither direction is imposed by current code. The
ascending direction is selected for the guest ABI; runtime tests remain
necessary to validate its implementation.

### Different RO and RC guest ABIs

Could reduce isolated-opcode harness overhead, but would create adapters and
risk validating behavior different from the compiler's. Use one guest ABI for
both. Preserve a distinct low-level `opcode_core` measurement/interface for
the existing arithmetic routines rather than creating a second guest ABI.

### Reimplement arithmetic inside stack operations

Would couple opcode correctness to duplicated arithmetic logic. Rejected;
invoke or generate the existing shared RV64 core implementation through
stack-aware wrappers.

### Execute a trap instruction to terminate guest code

Executing `EBREAK` or `ECALL` raises a typed rvemu exception; treating either
as success would conflate guest completion with an exception path and make
results executor-specific. Use a PC sentinel in the initial rvemu protocol.
Other executors may use their own internal mechanisms as long as they preserve
the guest result contract.

### Bind the guest ABI to one emulator

Would make EVM semantics, code generation, or the observable result depend on
rvemu-specific types and execution controls. Rejected in favor of a small
`Rv64Executor` boundary under RV64 infrastructure.

## 11. Executor Boundary and Emulator Choice

Keep **rvemu 0.0.11** as the primary emulator for current development. Unicorn
Engine and its Rust bindings are a possible future alternative, not a required
migration. The independent Unicorn investigation found the engine viable for
RV64 execution, memory/register access, execution control, and instrumentation.
Correctness, instruction-count comparability, and macOS ARM64 compatibility
still need experiments before any future backend decision.

Introduce a minimal `Rv64Executor` abstraction under RV64 infrastructure when
implementation begins. It should let the host load RV64 machine code, read
and write guest memory and registers, execute with an instruction limit,
observe completion and failures, and obtain instruction-count metrics with
explicit semantics. The initial implementation is rvemu-based. Do not design
a universal emulation framework or require identical internal execution
mechanisms across backends.

Keep responsibilities separate:

| Guest ABI | Executor interface |
| --- | --- |
| EVM state representation, stack layout and bounds, register conventions, state publication, observable result/exit contract. | Loading code, initializing emulator memory/registers, controlling execution, observing guest termination and failures, and collecting metrics. |

Executor-specific addresses, traps, hooks, and execution-control APIs are not
universal guest ABI requirements. The guest ABI must remain stable if the
executor implementation changes.

## 12. Open Questions

1. **Exception codes:** Assign stable guest exception-code values and document
   their encoding in the result record.
2. **Native stack:** Decide whether generated code needs the 64-KiB native
   stack reservation in the first runtime milestone; preserve its separation
   and applicable alignment if used.
3. **Future EVM state:** Define dynamic EVM memory, persistent storage, and
   gas support in later decisions.
4. **Executor interface:** Finalize the minimal `Rv64Executor` API during
   implementation without leaking rvemu types into EVM semantics or compiler
   contracts.
5. **Optional future executor research:** Unicorn compatibility, correctness,
   macOS ARM64 operation, hook/counting semantics, and metric comparability
   may be investigated before a backend or cross-engine comparison is
   considered. Such work is not required by this ADR and no Unicorn migration
   is authorized or implied.

The following architectural decisions are accepted and are not open design
questions: stack limit and direction, `s0`/`s1` assignments, depth derivation,
rvemu's nominal DRAM range and little-endian data accesses, code-size and
alignment constraints, the initial memory layout and exit PC, the result
record and publication protocol, the rvemu PC-sentinel protocol, the hybrid
result contract, and shared RO/RC ABI and opcode implementations.

## 13. ADR Acceptance and Implementation Validation Criteria

**ADR acceptance:** We agree on the architectural contract and authorize
implementation according to it.

**Milestone validation:** Runtime tests and benchmarks must later demonstrate
that the implementation conforms to the accepted contract. The complete
runtime does not need to exist before an architectural decision can be
accepted. The criteria below are obligations for the implementation phase;
they are not claims that these tests or benchmarks have already been run.

### Implementation Validation Criteria

1. Validate memory regions, code-size and access bounds, entry and exit
   addresses, and initialization/reuse behavior.
2. Validate register initialization, including `s0`, `s1`, and native `sp`
   when used.
3. Validate stack invariants, valid push/pop behavior, underflow/overflow
   handling, and that generated stack primitives enforce their bounds.
4. Validate result-record initialization, ordered publication, exit-boundary
   handling, status and pointer checks, and retrieval of the live stack.
5. Validate normal and exceptional termination, and distinguish guest errors,
   emulator errors, and instruction-limit exhaustion. Incomplete emulator
   state must never be returned as a valid EVM result.
6. Validate ADD/SUB stack integration, EVM operand ordering, exact destination
   alias behavior, and unsupported partial overlap.
7. Preserve historical `opcode_core` ADD/SUB benchmark compatibility, and
   record `opcode_guest`/`program_guest` boundaries and executor-specific
   instruction-count semantics.

The guest ABI, register convention, result layout, and executor boundary must
be implemented consistently for RO and RC. Runtime evidence must be based on
executed tests and benchmarks where applicable; source inspection alone is
not validation evidence. Exact exception-code assignments, immediate
native-stack necessity, and executor API details remain implementation
choices if they preserve the accepted decisions and observable contract.
Future Unicorn support and cross-engine metric equivalence are not
prerequisites.

The original implementation plan called for guest-state initialization, stack
access, and result retrieval first. It then called for guest-side `PUSH1`,
`ADD`, `SUB`, and `STOP`, including execution of `PUSH1 5; PUSH1 3; ADD; STOP`
as one complete RV64 program. See the implementation summary for status.

## 14. References to Existing Code

The `translator/` paths below refer to the pre-ADR prototype.

- `translator/src/lib.rs`: `Instruction`, `decode`, and `Opcode`; decoding
  accepts `PUSH1`–`PUSH32`, `ADD`, and `SUB`. `add_u256` and `sub_u256`
  implement little-endian four-limb arithmetic; generated RV64 routines use
  pointer arguments in `a0`–`a2` and current scratch registers `t0`–`t5`.
- `translator/src/execution.rs`: `Rv64Runner` loads code with
  `initialize_dram`, initializes operand memory and registers, sets the PC,
  steps `rvemu`, counts dynamic instructions, and retrieves the result.
  `ProgramRunner` maintains a Rust `Vec` stack and invokes the isolated
  arithmetic runners; it returns only the top item and errors on an empty
  result stack.
- `translator/src/benchmark.rs`: the current report describes executed
  decoded instructions and excludes the harness `EBREAK` sentinel; it also
  reports static routine instruction words separately. Its JSON identifies
  rvemu 0.0.11 as the emulator.
- `translator/README.md`: documents the present pointer calling convention,
  host-managed stack, emulator execution path, and the limits of the current
  experiment.
- `translator/Cargo.toml`: declares `rvemu = "0.0.11"`.
- [rvemu 0.0.11 `bus.rs`](https://docs.rs/crate/rvemu/0.0.11/source/src/bus.rs)
  and [`dram.rs`](https://docs.rs/crate/rvemu/0.0.11/source/src/dram.rs):
  DRAM base and size, bus address dispatch, byte layout, and DRAM
  initialization behavior.
- [rvemu 0.0.11 `cpu.rs`](https://docs.rs/crate/rvemu/0.0.11/source/src/cpu.rs)
  and [`exception.rs`](https://docs.rs/crate/rvemu/0.0.11/source/src/exception.rs):
  register and reset behavior, `cpu.execute()` results, and typed exception
  handling for breakpoints and environment calls.

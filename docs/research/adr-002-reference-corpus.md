# RF-028 — Cancun gas reference corpus for ADR-002

**Disposition.** Reference investigation complete. V01–V37 are proposed regression requirements for [ADR-002](../adr/002-cancun-evm-gas-metering.md). Human approval and Ferreth implementation are pending. The [TDD plan](adr-002-tdd.md) assigns each vector to an increment.

## Experiment and reproduction

At Ferreth v0.1.0 (`99544a588b4b6edabd74292db51b4f8488a`), run each program in a fresh empty frame on both pinned engines. The question was whether equivalent initial message budgets yield the same termination, settled gas and successful stacks across the seven-opcode milestone. Every row below was executed. All 74 processes exited zero with empty stderr. There were 19 successful frames per engine and 18 exceptional frames per engine.

On Darwin arm64, EELS used Python 3.14.5 and ethereum-execution 2.19.0 at `d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3`. The [EELS runner](../../research/eels-cancun/run_frame.py) invokes Cancun process_message with empty memory, zero value, no transfer and the supplied message gas. Optional --tx-gas defaults to message gas. An extra V29 control with --tx-gas 0 also returned [8, 6], remaining 6. REVM 43.0.3, interpreter 43.0.2 and handler 43.0.3 carry upstream revision `6b2e655b91f21c3a21770bed78def5b379ed1d17`. The [direct runner](../../research/revm-frame/src/main.rs) uses run_plain, fresh shared memory, default inputs, DummyHost and Cancun instruction pricing with an explicit u64 budget. Both CLIs begin with stack [] and bypass transaction processing. [EELS setup](../../research/eels-cancun/README.md) · [Cargo.lock](../../Cargo.lock)

From the repository root, with the pinned EELS environment provisioned and the locked Rust dependencies cached, use Rust/Cargo 1.98.1. For every table row run `pair HEX GAS`, passing an empty string for empty hex and expanding P as below.

```sh
revm_target=$(mktemp -d)
CARGO_TARGET_DIR="$revm_target" cargo build --frozen -p revm-frame
pair() {
  research/eels-cancun/.venv/bin/python -B research/eels-cancun/run_frame.py --code "$1" --gas "$2"
  "$revm_target/debug/revm-frame" --code "$1" --gas "$2"
}
prefix=$(research/eels-cancun/.venv/bin/python -B -c 'print("6001" * 1024)')
pair '' 0
pair 60020100 20
pair 5a5a00 10
pair "${prefix}5a00" 3074
```

## Observations and fixture expectations

Hex prefix P is exactly 1,024 copies of 6001 (PUSH1 1), costing 3,072 gas. F denotes a raw stack of exactly 1,024 ones. All other stacks list words bottom to top in decimal. Each reference cell gives raw outcome, stack, raw remaining, raw used and terminal PC in that order. For EELS, OK means Success=True and Error=None. G/U/O mean Success=False with OutOfGasError(), StackUnderflowError(), or StackOverflowError() respectively. For REVM, OK means Success=true, Result=Stop and Error=None. G/U/O mean Success=false with Result and Error both equal to OutOfGas, StackUnderflow or StackOverflow respectively. Raw used equals budget minus the runner's raw remaining. N/X in the last column mean normal/exceptional, followed by settled remaining and gross used. On N rows the common observed stack is also the fixture expectation. On X rows retain both raw stacks for diagnostics.

| ID | Hex | Initial gas | EELS | REVM | Portable fixture |
|---|---|---:|---|---|---|
| V01 | `00` | 0 | OK · [] · 0 · 0 · 1 | OK · [] · 0 · 0 · 1 | N · 0 · 0 |
| V02 | `005a` | 7 | OK · [] · 7 · 0 · 1 | OK · [] · 7 · 0 · 1 | N · 7 · 0 |
| V03 | `''` | 0 | OK · [] · 0 · 0 · 0 | OK · [] · 0 · 0 · 1 | N · 0 · 0 |
| V04 | `600200` | 2 | G · [] · 0 · 2 · 0 | G · [] · 0 · 2 · 1 | X · 0 · 2 |
| V05 | `600200` | 3 | OK · [2] · 0 · 3 · 3 | OK · [2] · 0 · 3 · 3 | N · 0 · 3 |
| V06 | `6002` | 4 | OK · [2] · 1 · 3 · 2 | OK · [2] · 1 · 3 · 3 | N · 1 · 3 |
| V07 | `60` | 3 | OK · [0] · 0 · 3 · 2 | OK · [0] · 0 · 3 · 3 | N · 0 · 3 |
| V08 | `60025000` | 4 | G · [] · 0 · 4 · 2 | G · [2] · 0 · 4 · 3 | X · 0 · 4 |
| V09 | `60025000` | 5 | OK · [] · 0 · 5 · 4 | OK · [] · 0 · 5 · 4 | N · 0 · 5 |
| V10 | `60025000` | 6 | OK · [] · 1 · 5 · 4 | OK · [] · 1 · 5 · 4 | N · 1 · 5 |
| V11 | `5000` | 1 | U · [] · 0 · 1 · 0 | G · [] · 0 · 1 · 1 | X · 0 · 1 |
| V12 | `60028000` | 5 | G · [2] · 0 · 5 · 2 | G · [2] · 0 · 5 · 3 | X · 0 · 5 |
| V13 | `60028000` | 6 | OK · [2, 2] · 0 · 6 · 4 | OK · [2, 2] · 0 · 6 · 4 | N · 0 · 6 |
| V14 | `60028000` | 7 | OK · [2, 2] · 1 · 6 · 4 | OK · [2, 2] · 1 · 6 · 4 | N · 1 · 6 |
| V15 | `8000` | 3 | U · [] · 0 · 3 · 0 | O · [] · 0 · 3 · 1 | X · 0 · 3 |
| V16 | `600260030100` | 8 | G · [] · 0 · 8 · 4 | G · [2, 3] · 0 · 8 · 5 | X · 0 · 8 |
| V17 | `600260030100` | 9 | OK · [5] · 0 · 9 · 6 | OK · [5] · 0 · 9 · 6 | N · 0 · 9 |
| V18 | `600260030100` | 10 | OK · [5] · 1 · 9 · 6 | OK · [5] · 1 · 9 · 6 | N · 1 · 9 |
| V19 | `01` | 0 | U · [] · 0 · 0 · 0 | G · [] · 0 · 0 · 1 | X · 0 · 0 |
| V20 | `01` | 2 | U · [] · 0 · 2 · 0 | G · [] · 0 · 2 · 1 | X · 0 · 2 |
| V21 | `01` | 3 | U · [] · 0 · 3 · 0 | U · [] · 0 · 3 · 1 | X · 0 · 3 |
| V22 | `60020100` | 20 | U · [] · 0 · 20 · 2 | U · [2] · 14 · 6 · 3 | X · 0 · 20 |
| V23 | `600260030300` | 8 | G · [] · 0 · 8 · 4 | G · [2, 3] · 0 · 8 · 5 | X · 0 · 8 |
| V24 | `600260030300` | 9 | OK · [1] · 0 · 9 · 6 | OK · [1] · 0 · 9 · 6 | N · 0 · 9 |
| V25 | `600260030300` | 10 | OK · [1] · 1 · 9 · 6 | OK · [1] · 1 · 9 · 6 | N · 1 · 9 |
| V26 | `5a00` | 0 | G · [] · 0 · 0 · 0 | G · [] · 0 · 0 · 1 | X · 0 · 0 |
| V27 | `5a00` | 1 | G · [] · 0 · 1 · 0 | G · [] · 0 · 1 · 1 | X · 0 · 1 |
| V28 | `5a00` | 2 | OK · [0] · 0 · 2 · 2 | OK · [0] · 0 · 2 · 2 | N · 0 · 2 |
| V29 | `5a5a00` | 10 | OK · [8, 6] · 6 · 4 · 3 | OK · [8, 6] · 6 · 4 · 3 | N · 6 · 4 |
| V30 | `5a00` | 18446744073709551615 | OK · [18446744073709551613] · 18446744073709551613 · 2 · 2 | OK · [18446744073709551613] · 18446744073709551613 · 2 · 2 | N · 18446744073709551613 · 2 |
| V31 | `5a00` | 9223372036854775808 | OK · [9223372036854775806] · 9223372036854775806 · 2 · 2 | OK · [9223372036854775806] · 9223372036854775806 · 2 · 2 | N · 9223372036854775806 · 2 |
| V32 | `P600200` | 3074 | G · F · 0 · 3074 · 2048 | G · F · 0 · 3074 · 2049 | X · 0 · 3074 |
| V33 | `P600200` | 3075 | O · F · 0 · 3075 · 2048 | O · F · 0 · 3075 · 2049 | X · 0 · 3075 |
| V34 | `P5a00` | 3073 | G · F · 0 · 3073 · 2048 | G · F · 0 · 3073 · 2049 | X · 0 · 3073 |
| V35 | `P5a00` | 3074 | O · F · 0 · 3074 · 2048 | O · F · 0 · 3074 · 2049 | X · 0 · 3074 |
| V36 | `P8000` | 3075 | O · F · 0 · 3075 · 2048 | O · F · 0 · 3075 · 2049 | X · 0 · 3075 |
| V37 | `600280015a506007035a` | 25 | OK · [3, 4] · 4 · 21 · 10 | OK · [3, 4] · 4 · 21 · 11 | N · 4 · 21 |

## Interpretation and limits

V01–V07 cover STOP, unreachable GAS, empty EOF and PUSH1 including missing data. V08–V15 cover POP and DUP1. V16–V25 cover ADD and SUB with prefixes, insufficient gas and missing operands. V26–V31 cover post-charge GAS and unsigned boundaries. V32–V36 distinguish a full stack with insufficient and sufficient gas. V37 exercises cumulative costs ending at EOF.

EELS pops before charging POP, ADD and SUB. REVM charges before dispatch. This explains V08/V16/V23 stack differences and V11/V19/V20 reason differences. For V15, REVM maps both failure conditions of stack.dup to StackOverflow. EELS reports underflow. EOF steps and pre-dispatch PC advancement also differ. Preserve all original PCs. [Pinned EELS stack](https://github.com/ethereum/execution-specs/blob/d15ecf40062fcfe20d23b7b5ff1c078bcbe251c3/src/ethereum/forks/cancun/vm/instructions/stack.py) · [REVM interpreter](https://github.com/bluealloy/revm/blob/6b2e655b91f21c3a21770bed78def5b379ed1d17/crates/interpreter/src/interpreter.rs)

V22 is the sole numeric gas normalization. EELS observed 0 left and 20 used after process_message caught the exception. Direct REVM observed 14 left and 6 used. For exceptional halts only, handler settlement spends remaining regular gas, giving 0 left and 20 gross used. **REVM settlement is source-derived. The full handler pipeline was not executed.** Preserve successful remaining gas as observed. REVERT requires its own classification outside this corpus. On these rules both engines agree on all 37 outcomes and all 19 successful stacks. Ferreth execution, guest diagnostics and publication remain unverified. [Pinned REVM settlement](https://github.com/bluealloy/revm/blob/6b2e655b91f21c3a21770bed78def5b379ed1d17/crates/handler/src/frame.rs) · [RF-002 through RF-004](ferreth-evm-conformance.md#rf-002--exceptional-halts-consume-remaining-frame-gas)

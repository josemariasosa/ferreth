# Ferreth Benchmark Runbook

## Overview

ADR-001 defines three measurement scopes. Each reports dynamically executed
RV64 instructions; none is an EVM gas count, CPU-cycle count, or elapsed-time
measurement.

| Scope | What it measures | Command |
| --- | --- | --- |
| `opcode_core` | The arithmetic RV64 routine operating on 256-bit values through its pointer-based core interface. This is the historical ADD/SUB baseline. | `bench --op ADD|SUB|ALL --cases N --format json` |
| `opcode_guest` | One isolated guest opcode, including guest stack handling and guest termination. | `bench guest --format json` |
| `program_guest` | One complete supported EVM bytecode program compiled and executed as one guest program. | `bench guest --format json` |

These scopes include different amounts of generated code and runtime work.
Do not compare their totals as if they were the same workload.

## Historical arithmetic benchmarks

Run commands from the workspace root:

```sh
cargo run -p ferreth-compiler -- bench --op ADD --cases 1000 --seed 42 --format json
cargo run -p ferreth-compiler -- bench --op SUB --cases 1000 --seed 42 --format json
cargo run -p ferreth-compiler -- bench --op ALL --cases 1000 --seed 42 --format json
```

The current CLI accepts the following options:

| Option | Required | Valid values and meaning |
| --- | --- | --- |
| `--op` | Yes | `ADD`, `SUB`, or `ALL` (`ALL` runs both operations). |
| `--cases` | Yes | A non-negative `usize` count of randomized operand pairs **per operation**. The report also includes 12 directed cases per operation. |
| `--seed` | No | An unsigned 64-bit seed, decimal or `0x`-prefixed. If omitted, the implementation default is `0x123456789abcdef0`. |
| `--format` | Yes | `json` only. |

`ALL` resets the random generator to the same seed for each operation, so ADD
and SUB receive the same ordered random operand pairs. JSON is the only stdout
payload; diagnostics go to stderr. If differential parity fails, the command
still writes the JSON evidence and exits unsuccessfully.

The versioned `rv64-arithmetic-baseline` JSON includes:

- `configuration`: seed, randomized cases per operation, random generator,
  emulator, EVM reference/specification, and dependency versions.
- `operations`: opcode and EVM byte, directed/random/total/passed/failed case
  counts, and RV64 measurements.
- `rv64`: executed cases, FNV-1a machine-code hash, static routine instruction
  words, per-case min/max/mean, and aggregate instruction categories.
- `failures`: structured case and error evidence; it is empty for a passing
  run.

### Checked-in historical baseline

The historical report is at
[`crates/ferreth-compiler/runs/rv64-arithmetic.json`](../../crates/ferreth-compiler/runs/rv64-arithmetic.json).
It records a seed of `42` and `1000` randomized cases per operation, in addition
to 12 directed cases. Reproduce it with:

```sh
cargo run -p ferreth-compiler -- bench --op ALL --cases 1000 --seed 42 --format json
```

| Operation | Successful cases | Dynamic instructions per case | Dynamic instructions total | Routine words | FNV-1a machine-code hash |
| --- | ---: | ---: | ---: | ---: | --- |
| ADD | 1,012 | 34 | 34,408 | 34 | `fnv1a64:0xf1fff32ff47962f3` |
| SUB | 1,012 | 34 | 34,408 | 34 | `fnv1a64:0x9f008f320eeb7a7b` |

The static routine size is 34 instruction words. Each successful case
executes all 34 words, excluding the harness `EBREAK` return sentinel, giving
`1,012 * 34 = 34,408` dynamic instructions per operation.

**Verified metadata difference:** the checked-in historical report records
`revm` version `19.7.0`; the current manifest and current CLI output record
`revm` version `43.0.3`. A new run with seed 42 and 1,000 randomized cases
matches the checked-in ADD/SUB case counts, 34-instruction counts, aggregate
totals, routine sizes, and hashes. The dependency metadata differs, so compare
the measurement fields rather than expecting the complete JSON files to be
byte-for-byte identical.

## Guest benchmarks

The current guest benchmark command is:

```sh
cargo run -p ferreth-compiler -- bench guest --format json
```

The equivalent `--format=json` spelling is also accepted. The JSON schema has
top-level `schema_version`, `experiment`, `executor`, `measurement`,
`opcode_guest`, and `program_guest` fields. Each measurement object contains
`input`, `status`, and `rv64`; `rv64` contains `opcode_body`,
`guest_termination`, and `total`.

The current output identifies `rvemu 0.0.11`, and all following statuses are
`normal`. These are historical reference values, verified against the current
command:

| Scope | Case | Opcode body | Guest termination | Total |
| --- | --- | ---: | ---: | ---: |
| `opcode_guest` | `PUSH1 0x05` | 10 | 7 | 17 |
| `opcode_guest` | `ADD 0x05 0x03` | 42 | 7 | 49 |
| `opcode_guest` | `SUB 0x05 0x03` | 42 | 7 | 49 |
| `program_guest` | `6005` | 10 | 7 | 17 |
| `program_guest` | `60056003` | 20 | 7 | 27 |
| `program_guest` | `6005600301` | 62 | 7 | 69 |
| `program_guest` | `6005600303` | 62 | 7 | 69 |
| `program_guest` | `600560030100` | 63 | 7 | 70 |

For `program_guest`, input strings are bytecode hex without a `0x` prefix.
`600560030100` is the `PUSH1 5; PUSH1 3; ADD; STOP` program; explicit STOP
adds one executed instruction compared with the same program without STOP.

## Output interpretation

- **Dynamic versus static:** dynamic counts are instructions actually executed
  by the guest. The arithmetic report's `routine_instruction_words` is the
  static size of the routine; it is separate from the aggregate dynamic count.
- **Opcode body versus termination:** guest metrics split the instructions in
  the opcode/program body from the shared guest termination publisher. Their
  sum is `total`.
- **Exit sentinel:** rvemu checks whether the PC is at the configured exit
  boundary before fetching the next instruction. The sentinel is therefore
  observed but is not executed or counted. Guest termination instructions
  leading to that sentinel are counted.
- **rvemu counting:** rvemu's executor increments the count only after
  `cpu.execute()` completes successfully. An execution fault or instruction
  limit is an error, not a successful instruction count.
- **Different scopes:** `opcode_core` counts the arithmetic routine with its
  direct pointer interface; `opcode_guest` includes guest stack work and
  termination; `program_guest` includes all compiled bodies in one full
  program and a shared termination path.
- **Different backends:** a future executor may define or collect instruction
  metrics differently. The current report names its executor, and counts from
  different executor backends should not be assumed directly comparable.

## Reproducing and comparing results

Never redirect a new benchmark into the checked-in report. The following
commands save a fresh baseline run to a temporary file, pretty-print its JSON,
and compare the measurement fields to the historical report. Run the block
from the workspace root; it requires `python3` only for inspection/comparison.

```sh
tmp_report="$(mktemp)"
trap 'rm -f "$tmp_report"' EXIT

cargo run -p ferreth-compiler -- \
  bench --op ALL --cases 1000 --seed 42 --format json \
  > "$tmp_report"

python3 -m json.tool "$tmp_report"

python3 - "$tmp_report" crates/ferreth-compiler/runs/rv64-arithmetic.json <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    current = json.load(stream)
with open(sys.argv[2], encoding="utf-8") as stream:
    historical = json.load(stream)

configuration_fields = (
    "seed",
    "random_cases_per_operation",
    "random_generator",
    "emulator",
    "evm_spec",
)
for field in configuration_fields:
    assert current["configuration"][field] == historical["configuration"][field], field

operation_fields = ("name", "cases")
rv64_fields = (
    "machine_code_fnv1a64",
    "routine_instruction_words",
    "instructions_per_case",
    "instruction_categories",
)
if len(current["operations"]) != len(historical["operations"]):
    raise SystemExit("operation counts differ")
for actual, reference in zip(current["operations"], historical["operations"]):
    for field in operation_fields:
        assert actual[field] == reference[field], (actual["name"], field)
    for field in rv64_fields:
        assert actual["rv64"][field] == reference["rv64"][field], (
            actual["name"],
            field,
        )

print("ADD/SUB benchmark measurements match the historical report.")
print(
    "revm metadata:",
    historical["configuration"]["dependencies"]["revm"],
    "(historical) vs",
    current["configuration"]["dependencies"]["revm"],
    "(current)",
)
PY
```

The comparison intentionally excludes dependency versions because the checked
in report predates the current `revm` dependency. To inspect just the guest
JSON, run the guest benchmark and pipe or redirect stdout as needed:

```sh
cargo run -p ferreth-compiler -- bench guest --format json
```

The CLI has no output-file option; shell redirection is the supported way to
capture reports. Use a new temporary path for each run and retain the
checked-in report unchanged.

## Regression checklist

- [ ] Run the historical arithmetic command and confirm ADD and SUB each pass
      1,012 cases at 34 instructions per case (34,408 total).
- [ ] Confirm the arithmetic machine-code hashes remain
      `0xf1fff32ff47962f3` for ADD and `0x9f008f320eeb7a7b` for SUB.
- [ ] Run `bench guest --format json` repeatedly and confirm the guest metrics
      remain reproducible.
- [ ] Confirm `6005600301` reports 69 instructions and explicit STOP in
      `600560030100` reports 70.
- [ ] Write reports to a temporary file; do not overwrite
      `crates/ferreth-compiler/runs/rv64-arithmetic.json`.
- [ ] Run all package tests with `cargo test -p ferreth-compiler`.

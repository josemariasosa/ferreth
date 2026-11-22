# LF-031 — Benchmark stdout with dtc unavailable

**Disposition.** Open. Recommend documenting LF-031 as a v0.1.0 limitation until a backend change is approved. Both benchmark commands produce a correct report after a diagnostic prefix, but fail the single-document stdout requirement.

## Reproduction and recorded observations

From the repository root, with `command -v dtc` returning no path, run the following in a shell. Use a fresh capture directory to preserve the originals.

```sh
capture=$(mktemp -d)
cargo run --frozen -q -p ferreth-conformance -- bench guest --format json >"$capture/guest.stdout" 2>"$capture/guest.stderr"
echo $? >"$capture/guest.exit"
cargo run --frozen -q -p ferreth-conformance -- bench --op ALL --cases 1000 --seed 42 --format json >"$capture/arithmetic.stdout" 2>"$capture/arithmetic.stderr"
echo $? >"$capture/arithmetic.exit"
```

| Command | Unmodified stdout | Warning pairs / prefix bytes | Unmodified stderr | Exit code |
|---|---|---:|---|---|
| Guest | 4,167 bytes | 15 / 2,445 | Empty (0 bytes) | 0 |
| Arithmetic | 1,827 bytes | 2 / 326 | Empty (0 bytes) | 0 |

| Command | SHA-256 of unmodified stdout |
|---|---|
| Guest | `5fe319d3b352fd0e800decf469b8b17553df9c3848359a9f77f0d363a1c123af` |
| Arithmetic | `f4f44b247a0d506779e306b2956d147ba4a216d67a8293295f641660a64c9f71` |

Both hashes match the [Phase 2.4 captures](../guest-linking/phase-2.4-public-cutover.md#benchmark-and-arithmetic-core-evidence).

Python's `json.loads` rejected each raw stream at character 0. For diagnosis only, we consumed consecutive exact matches of the following two lines (including both newlines) from the beginning of each capture.

```text
WARNING: failed to read a device tree binary: No such file or directory (os error 2)
WARNING: maybe need to install dtc commend `apt install device-tree-compiler`
```

Parsing the entire remainder succeeded, including the check for trailing data. The 1,722-byte guest and 1,501-byte arithmetic suffixes compared equal as complete JSON objects to the [guest baseline](../../runs/phase-0/guest-benchmarks.json) and [arithmetic baseline](../../runs/rv64-arithmetic.json) respectively. No JSON fields were normalized.

## Source and repair boundary

Ferreth calls `Emulator::new()` in [RvemuExecutor::new](../../crates/ferreth-compiler/src/rv64/executor/rvemu.rs). In the Cargo.lock-pinned rvemu 0.0.11 source, follow `src/emulator.rs:20` → `src/cpu.rs:254` → `src/bus.rs:66` → `src/rom.rs:139`. Each new emulator constructs a ROM. Its private device-tree helper writes rvemu.dts, launches dtc via Command::output(), then reads rvemu.dtb. Failure to launch dtc reaches Rom::new's error arm, which uses two println! calls and continues with an empty device-tree blob. Shell stderr redirection misses those writes to the Rust process's stdout.

The constructor exposes no configuration or diagnostic writer. A pre-existing DTB cannot bypass the unconditional compiler invocation. Installing dtc is an environmental workaround. Other file or launch errors can reach the same warning arm. Filtering the stream would couple evidence capture to diagnostic wording. Redirecting the host process's stdout would also affect unrelated writes. A dedicated subprocess with an independent report channel could preserve diagnostics, but requires a worker protocol and failure propagation beyond this focused repair.

The smallest source correction is to direct both existing backend messages to stderr with eprintln!, preserving the fallback and execution error paths. Adopting a patched dependency requires separate approval. The pending regression is to run both built CLI commands with dtc absent from PATH, parse all stdout, compare both complete baselines, and verify diagnostics on stderr and exit status. No production change or regression test was made in this investigation.

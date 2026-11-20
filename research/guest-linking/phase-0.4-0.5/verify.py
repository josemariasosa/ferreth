"""Reconcile the six focused runs and the workspace logs against the RED gate."""
import ast
import json
from pathlib import Path
import re
import sys

root = Path(sys.argv[1])
tests = Path(sys.argv[2])
test_ranges = {
    "repeated_push1_46_eof_completes_normally": None,
    "repeated_push1_46_stop_completes_normally": None,
    "repeated_push1_47_eof_completes_normally": 2056,
    "repeated_push1_47_stop_completes_normally": 2060,
    "repeated_push1_47_eof_full_stack_publishes_overflow": 2056,
    "repeated_push1_47_stop_full_stack_publishes_overflow": 2060,
}
summary_pattern = re.compile(
    r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out", re.M
)
target_pattern = re.compile(r"^\s*(?:Running (.*?) \(|Doc-tests (\S+)\s*$)", re.M)
result_pattern = re.compile(r"^test (\S+) \.\.\. (ok|FAILED)$", re.M)


def read_case(directory, name, kind):
    return (directory / f"{name}.{kind}").read_text()


def exit_status(directory, name):
    return int(read_case(directory, name, "exit"))


def test_output(name):
    output = read_case(tests, name, "stdout")
    log = read_case(tests, name, "stderr")
    summary = [tuple(map(int, row)) for row in summary_pattern.findall(output)]
    counts = [int(n) for n in re.findall(r"^running (\d+) tests?$", output, re.M)]
    targets = [unit or f"Doc-tests {doc}" for unit, doc in target_pattern.findall(log)]
    assert len(targets) == len(counts) == len(summary), name
    assert [sum(row[:4]) for row in summary] == counts, name
    return output, summary, counts, targets


focused = {}
for name, value in test_ranges.items():
    output, summary, counts, _ = test_output(name)
    want_exit = 0 if value is None else 101
    want_state = "ok" if value is None else "FAILED"
    assert exit_status(tests, name) == want_exit
    assert counts == [1] and result_pattern.findall(output) == [(name, want_state)]
    assert summary[0][:4] == ((1, 0, 0, 0) if value is None else (0, 1, 0, 0))
    if value is not None:
        diagnostic = f"RV64 assembly failed: ImmediateOverflow {{ value: {value}, min: -2048, max: 2047"
        assert f'compile repeated PUSH1 bytecode: Link {{ description: "{diagnostic}' in output
    focused[name] = {"exit": want_exit, "status": want_state, "rejected_jal_displacement": value}

workspace = {}
for name in ("workspace", "workspace-complete"):
    assert exit_status(tests, name) == 101
    output, summaries, counts, targets = test_output(name)
    keys = ["passed", "failed", "ignored", "measured", "filtered"]
    totals = dict(zip(keys, [sum(column) for column in zip(*summaries)]))
    totals["discovered"] = sum(counts)
    assert totals["passed"] + totals["failed"] == totals["discovered"]
    assert totals["failed"] == 4 and all(totals[k] == 0 for k in ("ignored", "filtered", "measured"))
    observed_failures = {name for name, outcome in result_pattern.findall(output) if outcome == "FAILED"}
    assert observed_failures == {name for name, value in test_ranges.items() if value is not None}
    # Independently count the individual test result lines, including controls.
    all_results = result_pattern.findall(output)
    assert sum(outcome == "ok" for _, outcome in all_results) == totals["passed"]
    assert len(all_results) == totals["discovered"]
    if name == "workspace-complete":
        assert totals == {"discovered": 141, "passed": 137, "failed": 4,
                          "ignored": 0, "measured": 0, "filtered": 0}
        assert totals["passed"] - 2 == 135
        assert all((test, "ok") in all_results for test, value in test_ranges.items() if value is None)
    workspace[name] = {"exit": 101, **totals, "targets": [
        {"target": target, **dict(zip(keys, summary))}
        for target, summary in zip(targets, summaries)
    ]}

for name in ("eels-eof", "eels-stop", "revm-eof", "revm-stop"):
    assert exit_status(root, name) == 0
    output = read_case(root, name, "stdout")
    stack = re.search(r"^Stack: (.+)$", output, re.M)
    assert ast.literal_eval(stack.group(1)) == [2] * 47
    assert re.search(r"^Success: true$", output, re.M | re.I)
    assert re.search(r"^(?:Raw remaining|Remaining) gas: 0$", output, re.M)
assert exit_status(root, "probe") == 0
assert exit_status(tests, "fmt") == 0
(root / "verification.json").write_text(json.dumps(
    {"focused": focused, "workspace": workspace, "formatting_exit": 0}, indent=2
) + "\n")
for name, result in workspace.items():
    print(name, {k: v for k, v in result.items() if k != "targets"})

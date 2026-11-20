"""Retain benchmark streams verbatim and assemble the Phase 0.3 baseline."""
import ast
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
specimens = root / "specimens"


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def benchmark(name, experiment):
    path = root / f"{name}.stdout"
    raw = path.read_text()
    try:
        report = json.loads(raw)
        warnings = []
        raw_valid = True
        offset = 0
    except json.JSONDecodeError:
        lines = raw.splitlines(keepends=True)
        start = next(i for i, line in enumerate(lines) if line.lstrip().startswith("{"))
        assert all(line.startswith("WARNING:") for line in lines[:start])
        offset = len("".join(lines[:start]).encode())
        report = json.loads("".join(lines[start:]))
        warnings = [line.rstrip("\r\n") for line in lines[:start]]
        raw_valid = False
    assert report["experiment"] == experiment
    write_json(root / f"{name}.parsed.json", report)
    exit_code = int((root / f"{name}.exit").read_text())
    assert exit_code == 0
    return report, {
        "exit_code": exit_code,
        "raw_stdout_is_json": raw_valid,
        "stdout_warning_line_count": len(warnings),
        "json_payload_byte_offset": offset,
        "stderr_bytes": (root / f"{name}.stderr").stat().st_size,
    }


def address(pc, entry):
    return {"pc": f"0x{pc:08x}", "entry_relative_offset": hex(pc - entry)}


def read_case(path):
    case_id = path.stem
    (evm, initial, final, status, exception, initial_s1, final_s1, depth,
     body, termination, total, selected, initial_sp, final_sp, entry, exit_pc
     ) = path.read_text().rstrip("\n").split("\t")
    status, exception, initial_s1, final_s1, depth, body, termination, total, selected, initial_sp, final_sp, entry, exit_pc = map(
        int, (status, exception, initial_s1, final_s1, depth, body, termination, total,
              selected, initial_sp, final_sp, entry, exit_pc)
    )
    initial_stack, final_stack = [text.split(",") if text else [] for text in (initial, final)]
    assert depth == len(final_stack)
    assert entry == 0x80000000 and exit_pc == 0x800ffffc
    assert initial_s1 == 0x80101000 + 32 * len(initial_stack)
    assert final_s1 == 0x80101000 + 32 * len(final_stack)
    artifact = path.with_suffix(".bin")
    code = artifact.read_bytes()
    assert len(code) % 4 == 0 and entry + len(code) <= exit_pc
    words = [f"0x{int.from_bytes(code[i:i+4], 'little'):08x}" for i in range(0, len(code), 4)]
    path.with_suffix(".words").write_text("\n".join(words) + "\n")
    labels, publishers = {}, {}
    for line in path.with_suffix(".labels.tsv").read_text().splitlines():
        label, pc, publisher = line.split("\t")
        labels[label] = address(int(pc), entry)
        if publisher:
            publishers[publisher] = labels[label]
    compiler_path = path.with_suffix(".compiler.tsv")
    compiler_metadata = None
    if compiler_path.exists():
        normal_count, exceptional_count, limit, publisher_pcs = compiler_path.read_text().split("\t")
        compiler_metadata = {
            "normal_termination_instruction_words": int(normal_count),
            "exceptional_termination_instruction_words": int(exceptional_count),
            "instruction_limit": int(limit),
            "termination_entry_pcs": [address(pc, entry) for pc in ast.literal_eval(publisher_pcs)],
        }
    selected_role = next(role for role, label_address in publishers.items()
                         if int(label_address["pc"], 16) == selected)
    assert body + termination == total
    assert (status, exception) == {"normal": (1, 0), "overflow": (2, 1), "underflow": (2, 2)}[selected_role]
    metadata_trace = []
    for line in path.with_suffix(".abi.tsv").read_text().splitlines():
        step, pc, raw_status, raw_code, raw_s1 = map(int, line.split("\t"))
        metadata_trace.append({
            "publisher_instructions_executed": step, "next_pc": hex(pc), "status": raw_status,
            "exception_code": raw_code, "published_s1": hex(raw_s1),
        })
    assert len(metadata_trace) == termination + 1
    return {
        "evm_bytecode_hex": evm,
        "assembly_file": f"specimens/{case_id}.s",
        "machine_code_file": f"specimens/{case_id}.bin",
        "machine_words_file": f"specimens/{case_id}.words",
        "machine_words_le_decoded": words,
        "machine_code_length_bytes": len(code),
        "static_rv64_instruction_words": len(words),
        "entry_pc": hex(entry),
        "exit_sentinel_pc": hex(exit_pc),
        "executable_end_exclusive": address(entry + len(code), entry),
        "publisher_pcs": publishers,
        "publisher_pc_source": "assembler labels checked against compiler metadata" if compiler_metadata else "assembler labels from returned assembly, with bytes verified by reassembly",
        "compiler_metadata": compiler_metadata,
        "assembly_labels": labels,
        "execution": {
            "outcome": "guest_terminal_result",
            "terminal_status": {1: "normal", 2: "guest_exception"}[status],
            "terminal_status_raw": status,
            "exception_code": exception,
            "initial_stack_bottom_to_top": initial_stack,
            "final_stack_bottom_to_top": final_stack,
            "initial_stack_depth": len(initial_stack),
            "final_stack_depth": depth,
            "complete_live_stack_preserved": initial_stack == final_stack,
            "initial_s1": hex(initial_s1),
            "final_s1": hex(final_s1),
            "initial_native_sp": hex(initial_sp),
            "final_native_sp": hex(final_sp),
            "taken_publisher": selected_role,
            "taken_publisher_pc": address(selected, entry),
            "observed_body_instructions": body,
            "observed_termination_instructions": termination,
            "observed_total_instructions": total,
            "publisher_metadata_replay": metadata_trace,
        },
    }


cases = {path.stem: read_case(path) for path in sorted(specimens.glob("*.tsv"))
         if path.stem.count(".") == 0}
assert len(cases) == 41

representative = {
    "rep_empty": (15, 0, 7, 7, [], 0),
    "rep_push": (26, 10, 7, 17, [2], 0),
    "rep_push_stop": (35, 11, 7, 18, [2], 0),
    "rep_two_push": (37, 20, 7, 27, [2, 3], 0),
    "rep_push_add_stop": (89, 63, 7, 70, [5], 0),
    "rep_add_stop": (67, 3, 8, 11, [], 2),
    "rep_stop": (24, 1, 7, 8, [], 0),
    "rep_add": (66, 42, 7, 49, [5], 0),
    "rep_sub": (66, 42, 7, 49, [1], 0),
}
for case_id, expected in representative.items():
    case = cases[case_id]
    run = case["execution"]
    assert (
        case["static_rv64_instruction_words"], run["observed_body_instructions"],
        run["observed_termination_instructions"], run["observed_total_instructions"],
        [int(value, 16) for value in run["final_stack_bottom_to_top"]], run["exception_code"],
    ) == expected, case_id

expected_publishers = {
    "rep_empty": (0, 0x1c, None), "rep_push": (0x2c, 0x48, None),
    "rep_push_stop": (0x30, 0x4c, 0x6c), "rep_two_push": (0x58, 0x74, None),
    "rep_push_add_stop": (0x108, 0x124, 0x144), "rep_add_stop": (0xb0, 0xcc, 0xec),
    "rep_stop": (0x04, 0x20, 0x40), "rep_add": (0xac, 0xc8, 0xe8),
    "rep_sub": (0xac, 0xc8, 0xe8),
}
for case_id, expected in expected_publishers.items():
    observed = cases[case_id]["publisher_pcs"]
    assert tuple(
        int(observed[role]["entry_relative_offset"], 16) if role in observed else None
        for role in ("normal", "overflow", "underflow")
    ) == expected, case_id


exceptional = {
    "exc_push_eof": (1, 1024, 4), "exc_push_stop": (1, 1024, 4),
    "exc_dup_full": (1, 1024, 5), "exc_dup_empty": (2, 0, 2),
    "exc_pop_empty": (2, 0, 2), "exc_add_empty": (2, 0, 3),
    "exc_add_one": (2, 1, 3),
}
for case_id, expected in exceptional.items():
    run = cases[case_id]["execution"]
    assert (run["exception_code"], run["final_stack_depth"], run["observed_body_instructions"]) == expected
    assert run["complete_live_stack_preserved"]
    assert run["observed_termination_instructions"] == 8
assert len(set(cases["exc_push_eof"]["execution"]["initial_stack_bottom_to_top"])) == 1024

two = cases["rep_two_push"]
entry = int(two["entry_pc"], 16)
push_pcs = [int(two["assembly_labels"][f"guest_push1_{i}"]["pc"], 16) for i in (0, 1)]
normal = int(two["publisher_pcs"]["normal"]["pc"], 16)
overflow = int(two["publisher_pcs"]["overflow"]["pc"], 16)
patches = []
for site in (push_pcs[1] - 4, normal - 4):
    assert (site - entry) % 4 == 0 and entry <= site < normal
    word_string = two["machine_words_le_decoded"][(site - entry) // 4]
    word = int(word_string, 16)
    assert word & 0x7f == 0x6f and (word >> 7) & 31 == 0
    immediate = (((word >> 31) & 1) << 20 | ((word >> 12) & 255) << 12
                 | ((word >> 20) & 1) << 11 | ((word >> 21) & 1023) << 1)
    displacement = (immediate ^ (1 << 20)) - (1 << 20)
    target = site + displacement
    assert target == overflow
    patches.append({"site": address(site, entry), "word": word_string, "rd": "x0",
                    "signed_displacement_bytes": displacement, "target": address(target, entry)})
assert [pc - entry for pc in [*push_pcs, normal, overflow]] == [0, 0x2c, 0x58, 0x74]
assert two["executable_end_exclusive"]["entry_relative_offset"] == "0x94"
assert [(p["site"]["entry_relative_offset"], p["word"], p["signed_displacement_bytes"]) for p in patches] == [
    ("0x28", "0x04c0006f", 76), ("0x54", "0x0200006f", 32)]
assert (specimens / "sequence_compiled_eq.txt").read_text().strip() == "true"

comparisons = []
for ordinary, isolated in [
    ("rep_two_push", "eq_sequence"), ("exc_push_eof", "eq_sequence_full"),
    ("rep_add", "eq_add"), ("rep_sub", "eq_sub"),
    ("exc_add_empty", "iso_add_empty"), ("exc_add_one", "iso_add_one"),
    ("eq_sub_empty", "iso_sub_empty"), ("eq_sub_one", "iso_sub_one"),
]:
    a, b = cases[ordinary], cases[isolated]
    ar, br = a["execution"], b["execution"]
    checks = {
        field: ar[field] == br[field] for field in (
            "initial_stack_bottom_to_top", "terminal_status", "exception_code", "final_stack_bottom_to_top",
            "final_s1", "observed_body_instructions", "observed_termination_instructions", "observed_total_instructions",
        )
    }
    assert all(checks.values())
    assembly_equal = (root / a["assembly_file"]).read_bytes() == (root / b["assembly_file"]).read_bytes()
    machine_equal = a["machine_words_le_decoded"] == b["machine_words_le_decoded"]
    publishers_equal = a["publisher_pcs"] == b["publisher_pcs"]
    expect_equal = isolated.startswith("eq_sequence")
    assert assembly_equal == machine_equal == publishers_equal == expect_equal
    assert (a["compiler_metadata"] == b["compiler_metadata"]) if expect_equal else b["compiler_metadata"] is None
    comparisons.append({
        "ordinary_bytecode_case": ordinary, "sequence_or_isolated_case": isolated,
        "guest_result_and_count_equalities": checks,
        "assembly_equal": assembly_equal, "machine_bytes_equal": machine_equal,
        "publisher_inventory_and_addresses_equal": publishers_equal,
        "compiled_metadata_equal": a["compiler_metadata"] == b["compiler_metadata"] if expect_equal else None,
        "static_instruction_words": [a["static_rv64_instruction_words"], b["static_rv64_instruction_words"]],
    })

core, core_capture = benchmark("core", "rv64-arithmetic-baseline")
guest, guest_capture = benchmark("guest", "rv64-guest-instruction-metrics")
assert core["configuration"]["seed"] == 42
assert core["configuration"]["random_cases_per_operation"] == 1000
assert core["configuration"]["evm_spec"] == "CANCUN"
assert [op["name"] for op in core["operations"]] == ["ADD", "SUB"]
for op, expected_hash in zip(core["operations"], ["f1fff32ff47962f3", "9f008f320eeb7a7b"]):
    assert op["cases"] == {"directed": 12, "random": 1000, "total": 1012, "passed": 1012, "failed": 0}
    assert not op["failures"]
    rv64 = op["rv64"]
    assert rv64["executed_cases"] == 1012 and rv64["routine_instruction_words"] == 34
    assert rv64["machine_code_fnv1a64"] == "fnv1a64:0x" + expected_hash
    assert rv64["instructions_per_case"] == {"min": 34, "max": 34, "mean": 34}
    assert rv64["instruction_categories"]["total_executed"] == 34408

guest_static = {}
op_mapping = ["bench_op_push", "bench_op_add", "bench_op_sub", "bench_op_pop", "bench_op_dup"]
op_inputs = ["PUSH1 0x05", "ADD 0x05 0x03", "SUB 0x05 0x03", "POP [0x00]", "DUP1 [0x00]"]
expected_bodies = {
    "opcode_guest": dict(zip(op_inputs, [10, 42, 42, 3, 14])),
    "program_guest": dict(zip(["6005", "60056003", "6005600301", "6005600303", "600560030100",
                               "600550", "600580", "60058001", "6005600350", "60058050"],
                              [10, 20, 62, 62, 63, 13, 24, 66, 23, 27])),
}
for scope in ("opcode_guest", "program_guest"):
    ids = op_mapping if scope == "opcode_guest" else ["bench_program_" + m["input"] for m in guest[scope]]
    assert [m["input"] for m in guest[scope]] == list(expected_bodies[scope])
    guest_static[scope] = []
    for measurement, case_id in zip(guest[scope], ids):
        case = cases[case_id]
        run = case["execution"]
        assert measurement["status"] == run["terminal_status"] == "normal"
        assert measurement["rv64"] == {
            "opcode_body": run["observed_body_instructions"],
            "guest_termination": run["observed_termination_instructions"],
            "total": run["observed_total_instructions"],
        }
        assert measurement["rv64"]["opcode_body"] == expected_bodies[scope][measurement["input"]]
        assert measurement["rv64"]["guest_termination"] == 7
        guest_static[scope].append({
            **measurement, "observer_case": case_id,
            "static_rv64_instruction_words": case["static_rv64_instruction_words"],
        })

write_json(root / "baseline.json", {
    "schema_version": 1,
    "phase": "0.3",
    "environment": {
        "revision": (root / "revision").read_text().strip(),
        "git_status_before": (root / "git.before").read_text().strip(),
        "git_status_after": (root / "git.after").read_text().strip(),
        "rustc": (root / "rustc.version").read_text().splitlines(),
        "cargo": (root / "cargo.version").read_text().strip(),
        "cargo_lock_git_blob": (root / "lock.blob").read_text().strip(),
        "compiler_dependency_tree": (root / "compiler.tree").read_text().splitlines(),
        "conformance_dependency_tree": (root / "conformance.tree").read_text().splitlines(),
        "dtc_path": (root / "dtc.path").read_text().strip() or None,
    },
    "exception_stack_seed": {"generator": "ferreth_compiler::execution::random_u256", "initial_seed": 42},
    "representative_case_ids": list(representative),
    "exceptional_case_ids": list(exceptional),
    "cases": cases,
    "two_push_linking": {"case_id": "rep_two_push", "patches": patches, "compiled_artifacts_equal": True},
    "entry_point_comparisons": comparisons,
    "benchmarks": {"core": core_capture, "guest": guest_capture, "guest_with_static_observations": guest_static},
})
print(f"Verified {len(cases)} observations, {len(comparisons)} API comparisons, and both benchmark captures.")

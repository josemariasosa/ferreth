use super::*;
use crate::rv64::memory::NATIVE_STACK_REGION;

const POP: u8 = 0x50;
const DUP1: u8 = 0x80;
const POP_UNDERFLOW: &[(u64, u32)] = &[(0, 0x63), (12, 0x6f)];
const DUP1_UNDERFLOW: &[(u64, u32)] = &[(0, 0x63), (56, 0x6f)];
const DUP1_OVERFLOW: &[(u64, u32)] = &[(0, 0x63), (4, 0x37), (8, 0x33), (12, 0x63), (60, 0x6f)];

pub(super) type FaultPath = (u64, &'static [(u64, u32)]);

fn observe_stack_ops(
    program: &CompiledRv64Program,
    initial: &[U256],
    publisher: u64,
    code: u64,
    fault: Option<FaultPath>,
) -> Observation {
    observe_guest(program, initial, publisher, code, fault, &[])
}

pub(super) fn observe_guest(
    program: &CompiledRv64Program,
    initial: &[U256],
    publisher: u64,
    code: u64,
    fault: Option<FaultPath>,
    calls: &[(u64, usize)],
) -> Observation {
    let mut executor = RvemuExecutor::new().unwrap();
    executor.load_code(program.machine_code()).unwrap();
    executor
        .write_memory(program.exit_pc(), &0x0010_0073_u32.to_le_bytes())
        .unwrap();
    initialize_guest_state(&mut executor).unwrap();
    for (slot, &word) in initial.iter().enumerate() {
        write_evm_word(&mut executor, slot, word).unwrap();
    }
    let base = EVM_STACK_REGION.start();
    let s1 = IntegerRegister::EVM_STACK_NEXT_FREE;
    executor.write_register(s1, base + initial.len() as u64 * EVM_WORD_SIZE);
    let sp = NATIVE_STACK_REGION.end() - 32;
    let reserved = [
        (IntegerRegister::EVM_STACK_BASE, base),
        (IntegerRegister::STACK_POINTER, sp),
    ]
    .into_iter()
    .chain([3, 4, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27].map(|index| {
        (
            IntegerRegister::new(index).unwrap(),
            0xa5a5_0000 | u64::from(index),
        )
    }));
    let ra = IntegerRegister::RETURN_ADDRESS;
    let mut expected_ra = 0x1234_5670;
    executor.write_register(ra, expected_ra);
    for (register, value) in reserved.clone() {
        executor.write_register(register, value);
    }

    let guard = [0xa5; 32];
    let native = [0x3c; 64];
    executor
        .write_memory(EVM_STACK_REGION.end(), &guard)
        .unwrap();
    executor.write_memory(base - 32, &guard).unwrap();
    executor.write_memory(sp - 32, &native).unwrap();

    let mut stops = program.termination_entry_pcs().to_vec();
    stops.push(program.exit_pc());
    let mut cursor = program.entry_pc();
    let mut call_prefix = 0;
    for &(offset, depth) in calls {
        let call = program.entry_pc() + offset + 5 * 4;
        let leaf = program.entry_pc() + offset + 9 * 4;
        let mut checkpoints = stops.clone();
        checkpoints.push(call);
        let prefix = executor
            .execute_until(cursor, &checkpoints, program.instruction_limit())
            .unwrap();
        assert_eq!(prefix.exit_pc, call);
        assert_eq!(executor.read_register(ra), expected_ra);
        assert_eq!(
            executor
                .execute(call, leaf, 1)
                .unwrap()
                .executed_instructions,
            1
        );
        expected_ra = call + 4;
        assert_eq!(executor.read_register(ra), expected_ra);
        let next_free = base + depth as u64 * EVM_WORD_SIZE;
        assert_eq!(executor.read_register(s1), next_free);
        for (index, slots_below) in [(10, 1), (11, 2), (12, 2)] {
            assert_eq!(
                executor.read_register(IntegerRegister::new(index).unwrap()),
                next_free - slots_below * EVM_WORD_SIZE
            );
        }
        assert_eq!(
            executor
                .execute(leaf, expected_ra, 34)
                .unwrap()
                .executed_instructions,
            34
        );
        assert_eq!(executor.read_register(ra), expected_ra);
        assert_eq!(executor.read_register(s1), next_free);
        cursor = expected_ra + 4;
        assert_eq!(
            executor
                .execute(expected_ra, cursor, 1)
                .unwrap()
                .executed_instructions,
            1
        );
        assert_eq!(executor.read_register(s1), next_free - EVM_WORD_SIZE);
        call_prefix += prefix.executed_instructions + 36;
    }
    let mut body = if let Some((start_offset, path)) = fault {
        let start = program.entry_pc() + start_offset;
        stops.push(start);
        let prefix = executor
            .execute_until(cursor, &stops, program.instruction_limit())
            .unwrap();
        assert_eq!(prefix.exit_pc, start);
        let before_s1 = executor.read_register(s1);
        let read_stack_allocation = |executor: &mut RvemuExecutor| {
            let mut bytes = vec![0; EVM_STACK_REGION.size() as usize + guard.len()];
            executor.read_memory(base, &mut bytes).unwrap();
            bytes
        };
        let before = read_stack_allocation(&mut executor);
        // Step only the failing opcode, with its expected instruction PCs and
        // opcode fields. The executed sequence contains no loads or stores.
        for (i, &(offset, opcode)) in path.iter().enumerate() {
            let pc = start + offset;
            let next = path
                .get(i + 1)
                .map_or(publisher, |&(offset, _)| start + offset);
            let at = (pc - program.entry_pc()) as usize;
            let word = u32::from_le_bytes(program.machine_code()[at..at + 4].try_into().unwrap());
            assert_eq!(word & 0x7f, opcode, "failing opcode instruction at {pc:#x}");
            assert_eq!(
                executor
                    .execute(pc, next, 1)
                    .unwrap_or_else(|error| panic!("step {pc:#x} -> {next:#x}: {error}")),
                ExecutionReport {
                    exit_pc: next,
                    executed_instructions: 1
                }
            );
        }
        assert_eq!(executor.read_register(s1), before_s1);
        assert_eq!(read_stack_allocation(&mut executor), before);
        ExecutionReport {
            exit_pc: publisher,
            executed_instructions: prefix.executed_instructions + path.len() as u64,
        }
    } else {
        executor
            .execute_until(cursor, &stops, program.instruction_limit())
            .unwrap()
    };
    body.executed_instructions += call_prefix;
    assert_eq!(body.exit_pc, publisher);
    let words = if code == 0 { 7 } else { 8 };
    let (termination, trace) = observe_publication(&mut executor, program, publisher, words);
    let guest = read_guest_result(&mut executor, &termination).unwrap();
    for (register, value) in reserved {
        assert_eq!(executor.read_register(register), value, "{register:?}");
    }
    assert_eq!(executor.read_register(s1), guest.final_s1());
    assert_eq!(executor.read_register(ra), expected_ra);
    let mut actual_guard = [0; 32];
    executor
        .read_memory(EVM_STACK_REGION.end(), &mut actual_guard)
        .unwrap();
    assert_eq!(actual_guard, guard);
    executor.read_memory(base - 32, &mut actual_guard).unwrap();
    assert_eq!(actual_guard, guard);
    let mut actual_native = [0; 64];
    executor.read_memory(sp - 32, &mut actual_native).unwrap();
    assert_eq!(actual_native, native);

    Observation {
        body,
        termination,
        trace,
        guest,
    }
}

fn assert_parity(
    input: &[u8],
    initial: &[U256],
    expected: &[U256],
    code: u64,
    body_words: u64,
    fault: Option<FaultPath>,
) {
    let candidate = compile(input).unwrap();
    let legacy = crate::compiler::compile_evm_bytecode_legacy(input).unwrap();
    let normal = legacy.termination_entry_pcs()[0];
    assert_eq!(
        legacy.termination_entry_pcs(),
        [normal, normal + 28, normal + 60]
    );
    assert_eq!(legacy.machine_code().len() as u64, normal - ENTRY_PC + 92);
    let has_overflow = input.iter().any(|&byte| byte != POP);
    let candidate_publishers = if has_overflow {
        vec![normal, normal + 28, normal + 60]
    } else {
        vec![normal, normal + 28]
    };
    assert_eq!(candidate.termination_entry_pcs(), candidate_publishers);
    assert_eq!(
        candidate.machine_code().len() as u64,
        normal - ENTRY_PC + if has_overflow { 92 } else { 60 }
    );
    println!(
        "input={input:02x?} initial_depth={} candidate/legacy publisher PCs={:x?}/{:x?}",
        initial.len(),
        candidate_publishers,
        legacy.termination_entry_pcs()
    );

    let (candidate_publisher, legacy_publisher) = match code {
        0 => (normal, normal),
        1 => (normal + 60, normal + 28),
        2 => (normal + 28, normal + 60),
        _ => unreachable!("unexpected exception code"),
    };
    let actual = observe_stack_ops(&candidate, initial, candidate_publisher, code, fault);
    let reference = observe_stack_ops(&legacy, initial, legacy_publisher, code, fault);
    assert_guest_result(&actual, expected, code, body_words);
    assert_guest_result(&reference, expected, code, body_words);
    assert_eq!(actual.guest, reference.guest);
    assert_eq!(actual.trace, reference.trace);
}

#[test]
fn pop_runtime_boundaries_match_legacy() {
    let words = full_stack();
    for depth in [1, 2, 1024] {
        assert_parity(&[POP], &words[..depth], &words[..depth - 1], 0, 3, None);
    }
    assert_parity(&[POP], &[], &[], 2, 2, Some((0, POP_UNDERFLOW)));
    assert_parity(&[POP, POP], &words[..2], &[], 0, 6, None);
    assert_parity(
        &[POP, POP, POP],
        &words[..2],
        &[],
        2,
        8,
        Some((32, POP_UNDERFLOW)),
    );
}

#[test]
fn dup1_runtime_boundaries_match_legacy() {
    let words = full_stack();
    for depth in [1, 1022, 1023] {
        let initial = &words[..depth];
        let mut expected = initial.to_vec();
        expected.push(words[depth - 1]);
        assert_parity(&[DUP1], initial, &expected, 0, 14, None);
    }
    assert_parity(&[DUP1], &[], &[], 2, 2, Some((0, DUP1_UNDERFLOW)));
    assert_parity(&[DUP1], &words, &words, 1, 5, Some((0, DUP1_OVERFLOW)));
    let initial = &words[..1022];
    let mut expected = initial.to_vec();
    expected.extend([words[1021]; 2]);
    assert_parity(&[DUP1, DUP1], initial, &expected, 0, 28, None);
    assert_parity(
        &[DUP1, DUP1, DUP1],
        initial,
        &expected,
        1,
        33,
        Some((128, DUP1_OVERFLOW)),
    );
}

#[test]
fn mixed_stack_operations_match_legacy() {
    let words = full_stack();
    // Each duplicate copies a full-width seeded value after the immediate
    // above it has been popped. Trailing operations would change this result.
    let success = [0x60, 7, POP, DUP1, 0x60, 8, POP, DUP1, 0, POP, DUP1];
    let canonical = &success[..9];
    assert_eq!(compile(&success), compile(canonical));
    assert_eq!(
        crate::compiler::compile_evm_bytecode_legacy(&success),
        crate::compiler::compile_evm_bytecode_legacy(canonical)
    );
    assert_parity(
        &success,
        &words[..2],
        &[words[0], words[1], words[1], words[1]],
        0,
        55,
        None,
    );

    // PUSH + DUP + POP + POP exhausts the stack; the next DUP fails before STOP.
    let underflow = [0x60, 9, DUP1, POP, POP, DUP1, 0];
    assert_parity(&underflow, &[], &[], 2, 32, Some((140, DUP1_UNDERFLOW)));

    // POP makes one slot available; DUP1 fills it with the preceding live word.
    let overflow = [POP, 0x60, 9, POP, DUP1, DUP1, 0];
    let mut expected = words[..1023].to_vec();
    expected.push(words[1022]);
    assert_parity(
        &overflow,
        &words,
        &expected,
        1,
        35,
        Some((140, DUP1_OVERFLOW)),
    );
}

#[test]
#[ignore = "assembles multi-megabyte sources at the POP/DUP1 capacity boundaries"]
fn pop_dup1_capacity_probe() {
    use crate::rv64::memory::EXIT_PC;

    // These smaller legacy specimens already reach the signed 12-bit
    // numeric parser ceiling, preventing capacity-scale runtime comparisons.
    for (opcode, last, displacement) in [(POP, 124, 2048), (DUP1, 31, 2052)] {
        crate::compiler::compile_evm_bytecode_legacy(&vec![opcode; last]).unwrap();
        let CompileError::Link { description } =
            crate::compiler::compile_evm_bytecode_legacy(&vec![opcode; last + 1]).unwrap_err()
        else {
            panic!("expected legacy numeric JAL error");
        };
        assert!(
            description.contains(&format!(
                "ImmediateOverflow {{ value: {displacement}, min: -2048, max: 2047"
            )),
            "{description}"
        );
        println!(
            "legacy opcode={opcode:02x} count={} {description}",
            last + 1
        );
    }

    let pop_fragments = |count| {
        (0..count)
            .map(|index| {
                let continuation = if index + 1 == count {
                    PublisherRole::Normal.label().to_owned()
                } else {
                    format!("guest_pop_{}", index + 1)
                };
                generate_symbolic_pop(index, continuation)
            })
            .collect::<Vec<_>>()
    };
    let fragments = pop_fragments(49_999);
    let publishers = publisher_inventory(&fragments);
    let source = emit(&fragments, &publishers);
    let image = assemble_image(&source).unwrap();
    assert_eq!(image.labels().len(), 100_000);
    assert_eq!(image.len(), 800_044);
    println!(
        "POP count=49999 source_bytes={} labels={} image_bytes={} accepted",
        source.len(),
        image.labels().len(),
        image.len()
    );
    let candidate = verified_program(&fragments, &publishers, source, image).unwrap();
    let full = full_stack();
    let result = observe_stack_ops(
        &candidate,
        &full,
        ENTRY_PC + 800_012,
        2,
        Some((16 * 1024, POP_UNDERFLOW)),
    );
    assert_guest_result(&result, &[], 2, 3 * 1024 + 2);

    for count in [50_000, 65_532, 65_533] {
        let source = emit(&pop_fragments(count), &publishers);
        assert_label_limit(&source, "POP", count, 2 * count + 2, 16 * count + 60);
    }
    assert_eq!(EXIT_PC - ENTRY_PC, 1_048_572);
    assert_eq!(16 * 65_532 + 60, 1_048_572);
    validate_generated_code(ENTRY_PC, 16 * 65_532 + 60).unwrap();
    assert_eq!(
        validate_generated_code(ENTRY_PC, 16 * 65_533 + 60),
        Err(LayoutError::CodeOverlapsExitSlot {
            code_end: EXIT_PC + 16
        })
    );

    for (count, accepted) in [(16_382, true), (16_383, false)] {
        let fragments = dup1_fragments(count);
        let publishers = publisher_inventory(&fragments);
        let source = emit(&fragments, &publishers);
        let image = assemble_image(&source).unwrap();
        let size = 64 * count + 92;
        let normal = ENTRY_PC + 64 * count as u64;
        assert_eq!(image.len(), size);
        assert_eq!(image.labels().len(), 3 * count + 3);
        assert_eq!(image.label_address("dup1_underflow_0"), Some(ENTRY_PC + 56));
        assert_eq!(
            image.label_address("binary_stack_underflow_exit"),
            Some(normal + 28)
        );
        assert_eq!(
            image.label_address("push1_stack_overflow_exit"),
            Some(normal + 60)
        );
        println!(
            "DUP1 count={count} source_bytes={} labels={} image_bytes={size} accepted={accepted}",
            source.len(),
            image.labels().len()
        );
        let result = verified_program(&fragments, &publishers, source, image);
        if accepted {
            let candidate = result.unwrap();
            let underflow =
                observe_stack_ops(&candidate, &[], normal + 28, 2, Some((0, DUP1_UNDERFLOW)));
            assert_guest_result(&underflow, &[], 2, 2);
            let overflow =
                observe_stack_ops(&candidate, &full, normal + 60, 1, Some((0, DUP1_OVERFLOW)));
            assert_guest_result(&overflow, &full, 1, 5);
            let repeated = observe_stack_ops(
                &candidate,
                &full[..1],
                normal + 60,
                1,
                Some((64 * 1023, DUP1_OVERFLOW)),
            );
            assert_guest_result(&repeated, &vec![full[0]; 1024], 1, 14 * 1023 + 5);
        } else {
            assert_eq!(
                result,
                Err(CompileError::Layout(LayoutError::CodeOverlapsExitSlot {
                    code_end: ENTRY_PC + size as u64,
                }))
            );
        }
    }

    for (count, source_label, target, offset, line, disp) in [
        (
            16_384,
            "dup1_overflow_0",
            "push1_stack_overflow_exit",
            60,
            19,
            1_048_576,
        ),
        (
            33_331,
            "dup1_underflow_0",
            "binary_stack_underflow_exit",
            56,
            17,
            2_133_156,
        ),
        (
            33_332,
            "dup1_underflow_0",
            "binary_stack_underflow_exit",
            56,
            17,
            2_133_220,
        ),
    ] {
        let fragments = dup1_fragments(count);
        let source = emit(&fragments, &publisher_inventory(&fragments));
        assert_eq!(
            source.lines().nth(line - 2).unwrap(),
            format!("{source_label}:")
        );
        assert_eq!(
            source.lines().nth(line - 1).unwrap().trim(),
            format!("jal zero, {target}")
        );
        let publisher_offset = if offset == 56 { 28 } else { 60 };
        let site_pc = ENTRY_PC + offset;
        let target_pc = ENTRY_PC + (64 * count + publisher_offset) as u64;
        assert_eq!(target_pc - site_pc, disp);
        let CompileError::Link { description } = assemble_image(&source).unwrap_err() else {
            panic!("expected link-stage JAL range error");
        };
        assert!(description.contains(&format!(
            "BranchOutOfRange {{ label: \"{target}\", disp: {disp}, max: 1048574, span: Span {{ line: {line}, col: 5,"
        )), "{description}");
        println!("DUP1 count={count} source_bytes={} nominal_image_bytes={} site={site_pc:#x} target={target_pc:#x} {description}",
            source.len(), 64 * count + 92);
    }

    let count = 33_333;
    let fragments = dup1_fragments(count);
    let source = emit(&fragments, &publisher_inventory(&fragments));
    assert_label_limit(&source, "DUP1", count, 3 * count + 3, 64 * count + 92);
}

fn dup1_fragments(count: usize) -> Vec<SymbolicFragment> {
    (0..count)
        .map(|index| {
            let continuation = if index + 1 == count {
                PublisherRole::Normal.label().to_owned()
            } else {
                format!("guest_dup1_{}", index + 1)
            };
            generate_symbolic_dup1(index, continuation)
        })
        .collect()
}

fn assert_label_limit(source: &str, name: &str, count: usize, labels: usize, size: usize) {
    let CompileError::Link { description } = assemble_image(source).unwrap_err() else {
        panic!("expected assembler resource limit");
    };
    assert!(
        description.contains(r#"ResourceLimitExceeded { resource: "labels", limit: 100000 }"#),
        "{description}"
    );
    println!("{name} count={count} source_bytes={} nominal_labels={labels} nominal_image_bytes={size} {description}", source.len());
}

#[test]
fn fixed_layout_rejects_swapped_dup1_publishers_even_with_matching_declarations() {
    let mut fragments = [
        generate_symbolic_dup1(0, "guest_pop_1".to_owned()),
        generate_symbolic_pop(1, PublisherRole::Normal.label().to_owned()),
    ];
    let publishers = publisher_inventory(&fragments);
    let source = emit(&fragments, &publishers);
    let image = assemble_image(&source).unwrap();
    verified_program(&fragments, &publishers, source.clone(), image).unwrap();

    let broken = source.replace(
        "dup1_underflow_0:\n    jal zero, binary_stack_underflow_exit\ndup1_overflow_0:\n    jal zero, push1_stack_overflow_exit",
        "dup1_underflow_0:\n    jal zero, push1_stack_overflow_exit\ndup1_overflow_0:\n    jal zero, binary_stack_underflow_exit",
    );
    assert_ne!(broken, source);
    let assert_role_error = |fragments: &[SymbolicFragment]| {
        let image = assemble_image(&broken).unwrap();
        let error = verified_program(fragments, &publishers, broken.clone(), image).unwrap_err();
        assert!(
            matches!(&error, CompileError::Link { description } if description.contains(
                "transfer at 0x80000038 targets 0x8000008c, expected 0x8000006c"
            )),
            "{error:?}"
        );
    };
    assert_role_error(&fragments);
    // With matching declarations the scan accepts those targets, but the
    // independent fixed-layout gate still requires the original roles.
    fragments[0].external_references[0].role = PublisherRole::Overflow;
    fragments[0].external_references[1].role = PublisherRole::Underflow;
    assert_role_error(&fragments);
}

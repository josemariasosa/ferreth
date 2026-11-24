use super::*;
use crate::rv64::{guest::write_evm_word, memory::NATIVE_STACK_REGION};

const POP: u8 = 0x50;
const DUP1: u8 = 0x80;
const POP_UNDERFLOW: &[(u64, u32)] = &[(0, 0x63), (12, 0x6f)];
const DUP1_UNDERFLOW: &[(u64, u32)] = &[(0, 0x63), (56, 0x6f)];
const DUP1_OVERFLOW: &[(u64, u32)] = &[(0, 0x63), (4, 0x37), (8, 0x33), (12, 0x63), (60, 0x6f)];

pub(super) type FaultPath<'a> = (u64, &'a [(u64, u32)]);

fn observe_stack_ops(
    program: &CompiledRv64Program,
    initial: &[U256],
    publisher: u64,
    code: u64,
    fault: Option<FaultPath<'_>>,
) -> Observation {
    observe_guest(program, initial, publisher, code, fault, &[], 0)
}

pub(super) fn observe_guest(
    program: &CompiledRv64Program,
    initial: &[U256],
    publisher: u64,
    code: u64,
    fault: Option<FaultPath<'_>>,
    calls: &[(u64, usize)],
    limit: u64,
) -> Observation {
    let mut executor = RvemuExecutor::new().unwrap();
    executor.load_code(program.machine_code()).unwrap();
    executor
        .write_memory(program.exit_pc(), &0x0010_0073_u32.to_le_bytes())
        .unwrap();
    initialize_guest_state(&mut executor, limit).unwrap();
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
        // opcode fields, then check the full stack allocation is unchanged.
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
    let words = if code == 0 { 9 } else { 10 };
    let (termination, trace) = observe_publication(&mut executor, program, publisher, words);
    let guest = read_guest_result(&mut executor, &termination, limit).unwrap();
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
    fault: Option<FaultPath<'_>>,
) {
    let candidate = compile(input).unwrap();
    let legacy = crate::compiler::compile_evm_bytecode_legacy(input).unwrap();
    let normal = legacy.termination_entry_pcs()[0];
    assert_eq!(
        legacy.termination_entry_pcs(),
        [normal, normal + 36, normal + 76]
    );
    assert_eq!(legacy.machine_code().len() as u64, normal - ENTRY_PC + 116);
    let charges = legacy_charge_sites(input);
    let metered_normal = normal + 32 * charges.len() as u64;
    let has_overflow = decode_detailed(input).unwrap().iter().any(|op| {
        matches!(
            op,
            DecodedInstruction::Dup1 { .. } | DecodedInstruction::Push { .. }
        )
    });
    let mut publishers = vec![metered_normal, metered_normal + 36];
    if has_overflow {
        publishers.push(metered_normal + 76);
    }
    publishers.push(publishers.last().unwrap() + 40);
    assert_eq!(candidate.termination_entry_pcs(), publishers);
    assert_eq!(
        candidate.machine_code().len() as u64,
        publishers.last().unwrap() - ENTRY_PC + 40
    );

    let (candidate_publisher, legacy_publisher) = match code {
        0 => (metered_normal, normal),
        1 => (metered_normal + 76, normal + 36),
        2 => (metered_normal + 36, normal + 76),
        _ => unreachable!("unexpected exception code"),
    };
    let funded: Vec<_> = charges
        .iter()
        .take_while(|&&(site, _)| fault.is_none_or(|(failure, _)| site <= failure))
        .collect();
    let executed = funded.len() as u64;
    let budget = funded.iter().map(|&&(_, cost)| cost).sum();
    let metered_path = fault.map(|(_, path)| funded_path(path));
    let metered_fault = fault
        .zip(metered_path.as_deref())
        .map(|((site, _), path)| (site + 32 * (executed - 1), path));
    let actual = observe_guest(
        &candidate,
        initial,
        candidate_publisher,
        code,
        metered_fault,
        &[],
        budget,
    );
    let reference = observe_stack_ops(&legacy, initial, legacy_publisher, code, fault);
    assert_guest_result(&actual, expected, code, body_words + 7 * executed);
    assert_guest_result(&reference, expected, code, body_words);
    assert_eq!(actual.guest.gas_limit(), budget);
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
    for (opcode, last, displacement) in [(POP, 123, 2048), (DUP1, 31, 2068)] {
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

    assert_eq!(EXIT_PC - ENTRY_PC, 1_048_572);
    let full = full_stack();
    for (opcode, count, stop, size, accepted) in [
        (POP, 21_842, false, 1_048_532, true),
        (POP, 21_843, false, 1_048_580, false),
        (POP, 21_842, true, 1_048_536, true),
        (POP, 21_843, true, 1_048_584, false),
        (DUP1, 10_921, false, 1_048_572, true),
        (DUP1, 10_920, true, 1_048_480, true),
        (DUP1, 10_921, true, 1_048_576, false),
    ] {
        let source = stack_capacity_source(opcode, count, stop);
        let image = assemble_image(&source).unwrap();
        let label_count = if opcode == POP {
            3 * count + 3
        } else {
            4 * count + 4
        };
        assert_eq!(image.labels().len(), label_count + usize::from(stop));
        assert_eq!(image.len(), size);

        let normal = ENTRY_PC
            + if opcode == POP {
                size - 116
            } else {
                size - 156
            } as u64;
        let oog = ENTRY_PC + size as u64 - 40;
        assert_eq!(image.label_address("normal_exit"), Some(normal));
        assert_eq!(image.label_address("out_of_gas_exit"), Some(oog));
        println!(
            "opcode={opcode:02x} count={count} stop={stop} source_bytes={} labels={} image_bytes={size} first_oog_disp={} accepted={accepted}",
            source.len(), image.labels().len(), oog - (ENTRY_PC + 20),
        );

        let mut input = vec![opcode; count];
        input.extend(stop.then_some(0));
        let result = compile(&input);
        if accepted {
            let candidate = result.unwrap();
            assert_eq!(candidate.machine_code(), image.bytes());
            let oog_result =
                observe_guest(&candidate, &[], oog, 3, Some((0, OOG_CHARGE_PATH)), &[], 0);
            assert_guest_result(&oog_result, &[], 3, 6);
            if opcode == POP {
                let path = funded_path(POP_UNDERFLOW);
                let result = observe_guest(
                    &candidate,
                    &full,
                    normal + 36,
                    2,
                    Some((48 * 1024, &path)),
                    &[],
                    2 * 1025,
                );
                assert_guest_result(&result, &[], 2, 10 * 1024 + 9);
            } else {
                let empty_path = funded_path(DUP1_UNDERFLOW);
                let underflow = observe_guest(
                    &candidate,
                    &[],
                    normal + 36,
                    2,
                    Some((0, &empty_path)),
                    &[],
                    3,
                );
                assert_guest_result(&underflow, &[], 2, 9);
                let full_path = funded_path(DUP1_OVERFLOW);
                let overflow = observe_guest(
                    &candidate,
                    &full,
                    normal + 76,
                    1,
                    Some((0, &full_path)),
                    &[],
                    3,
                );
                assert_guest_result(&overflow, &full, 1, 12);
                let repeated = observe_guest(
                    &candidate,
                    &full[..1],
                    normal + 76,
                    1,
                    Some((96 * 1023, &full_path)),
                    &[],
                    3 * 1024,
                );
                assert_guest_result(&repeated, &vec![full[0]; 1024], 1, 21 * 1023 + 12);
            }
        } else {
            assert_eq!(
                result,
                Err(CompileError::Layout(LayoutError::CodeOverlapsExitSlot {
                    code_end: ENTRY_PC + size as u64,
                }))
            );
        }
    }

    // The first OOG site is at +20 and its shared publisher starts
    // 40 bytes before the nominal end. Test relocation independently
    // from the ABI image check and the assembler label ceiling.
    for (opcode, count, stop, size) in [
        (POP, 21_844, false, 1_048_628),
        (POP, 21_844, true, 1_048_632),
    ] {
        let source = stack_capacity_source(opcode, count, stop);
        let image = assemble_image(&source).unwrap();
        assert_eq!(image.len(), size);
        assert_eq!(
            image.label_address("out_of_gas_exit"),
            Some(ENTRY_PC + size as u64 - 40)
        );
        println!(
            "POP count={count} stop={stop} image_bytes={size} first_oog_disp={}",
            size - 60
        );
    }
    for (opcode, count, stop, size) in [
        (POP, 21_845, false, 1_048_676),
        (POP, 21_845, true, 1_048_680),
        (DUP1, 10_922, false, 1_048_668),
        (DUP1, 10_922, true, 1_048_672),
        (POP, 33_332, false, 48 * 33_332 + 116),
        (DUP1, 24_999, false, 96 * 24_999 + 156),
    ] {
        let source = stack_capacity_source(opcode, count, stop);
        let CompileError::Link { description } = assemble_image(&source).unwrap_err() else {
            panic!("expected link-stage JAL range error");
        };
        let disp = size - 60;
        assert!(
            description.contains(&format!(
                "BranchOutOfRange {{ label: \"out_of_gas_exit\", disp: {disp}, max: 1048574"
            )),
            "{description}"
        );
        println!("opcode={opcode:02x} count={count} stop={stop} nominal_image_bytes={size} {description}");
    }
    for (opcode, count, stop, labels, size) in [
        (POP, 33_333, false, 100_002, 48 * 33_333 + 116),
        (DUP1, 25_000, false, 100_004, 96 * 25_000 + 156),
        (DUP1, 24_999, true, 100_001, 96 * 24_999 + 160),
    ] {
        let source = stack_capacity_source(opcode, count, stop);
        let name = if opcode == POP { "POP" } else { "DUP1" };
        assert_label_limit(&source, name, count, labels, size);
    }

    for opcode in [0x01, 0x03] {
        for stop in [false, true] {
            let mut input = vec![opcode; 5_139];
            input.extend(stop.then_some(0));
            let size = 1_048_472 + 4 * usize::from(stop);
            let candidate = compile(&input).unwrap();
            assert_eq!(candidate.machine_code().len(), size);
            let image = assemble_image(candidate.assembly()).unwrap();
            let labels = 5 * 5_139 + 3 + usize::from(stop);
            assert_eq!(image.labels().len(), labels);
            let target = ENTRY_PC + size as u64 - 40;
            let observed = observe_guest(
                &candidate,
                &[],
                target,
                3,
                Some((0, OOG_CHARGE_PATH)),
                &[],
                0,
            );
            assert_guest_result(&observed, &[], 3, 6);

            input.insert(0, opcode);
            let CompileError::Link { description } = compile(&input).unwrap_err() else {
                panic!("expected arithmetic OOG JAL range error");
            };
            let displacement = size + 204 - 60;
            assert!(
                description.contains(&format!(
                "BranchOutOfRange {{ label: \"out_of_gas_exit\", disp: {displacement}, max: 1048574"
            )),
                "{description}"
            );
            println!(
                "opcode={opcode:02x} count=5139 stop={stop} image_bytes={size} labels={labels}; count=5140 {description}"
            );
        }
    }
}

fn stack_capacity_source(opcode: u8, count: usize, stop: bool) -> String {
    let (generate, entry): (fn(usize, String) -> SymbolicFragment, _) = match opcode {
        POP => (generate_symbolic_pop, "guest_pop"),
        DUP1 => (generate_symbolic_dup1, "guest_dup1"),
        _ => unreachable!(),
    };
    let fragments: Vec<_> = (0..count)
        .map(|index| {
            let continuation = if index + 1 < count {
                format!("{entry}_{}", index + 1)
            } else if stop {
                "guest_stop".to_owned()
            } else {
                PublisherRole::Normal.label().to_owned()
            };
            generate(index, continuation)
        })
        .chain(stop.then(stop_fragment))
        .collect();
    emit(&fragments, &publisher_inventory(&fragments))
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
                "transfer at 0x80000058 targets 0x800000dc, expected 0x800000b4"
            )),
            "{error:?}"
        );
    };
    assert_role_error(&fragments);
    // With matching declarations the scan accepts those targets, but the
    // independent fixed-layout gate still requires the original roles.
    fragments[0].external_references[1].role = PublisherRole::Overflow;
    fragments[0].external_references[2].role = PublisherRole::Underflow;
    assert_role_error(&fragments);
}

#[test]
fn full_stack_dup1_preserves_v36_allocation_and_settles_live_gas() {
    let mut input = [0x60, 1].repeat(1024);
    input.extend([DUP1, 0]);
    let program = compile(&input).unwrap();
    let publishers = program.termination_entry_pcs();
    let overflow = funded_path(DUP1_OVERFLOW);
    for (limit, code, publisher, path, residual) in [
        (3074, 3, publishers[3], OOG_CHARGE_PATH, 2),
        (3075, 1, publishers[2], overflow.as_slice(), 0),
        (3080, 1, publishers[2], overflow.as_slice(), 5),
    ] {
        let observed = observe_guest(
            &program,
            &[],
            publisher,
            code,
            Some((1024 * 76, path)),
            &[],
            limit,
        );
        assert_eq!(
            observed.guest.status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(observed.guest.exception_code(), code);
        assert_eq!(observed.guest.stack(), &[U256::from(1); 1024]);
        assert_eq!(observed.guest.final_s1(), EVM_STACK_REGION.end());
        assert_eq!(observed.guest.remaining_gas(), 0);
        assert_eq!(observed.guest.gas_used(), limit);
        assert_eq!(
            observed.body.executed_instructions,
            1024 * 17 + path.len() as u64
        );
        assert_eq!(observed.termination.executed_instructions, 10);
        let running = |live| {
            GuestMetadata::new(
                GuestExecutionStatus::Running,
                code,
                EVM_STACK_REGION.end(),
                limit,
                live,
                0,
            )
            .encode()
        };
        assert_eq!(observed.trace[5], running(residual));
        assert_eq!(observed.trace[6..9], [running(0); 3]);
    }
}

#[test]
fn unfunded_pop_and_dup1_preserve_stack_allocation() {
    for (opcode, limit) in [(POP, 4), (DUP1, 5)] {
        let program = compile(&[0x60, 2, opcode, 0]).unwrap();
        let observed = observe_guest(
            &program,
            &[],
            *program.termination_entry_pcs().last().unwrap(),
            3,
            Some((76, OOG_CHARGE_PATH)),
            &[],
            limit,
        );
        assert_eq!(
            observed.guest.status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(observed.guest.exception_code(), 3);
        assert_eq!(observed.guest.stack(), &[U256::from(2)]);
        assert_eq!(
            observed.guest.final_s1(),
            EVM_STACK_REGION.start() + EVM_WORD_SIZE
        );
        assert_eq!(observed.guest.remaining_gas(), 0);
        assert_eq!(observed.guest.gas_used(), limit);
        assert_eq!(observed.body.executed_instructions, 17 + 6);
        assert_eq!(observed.termination.executed_instructions, 10);
    }
}

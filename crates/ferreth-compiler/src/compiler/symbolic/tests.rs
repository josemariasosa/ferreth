use alloy_primitives::U256;

use super::*;
use crate::{
    compiler::{compile_evm_bytecode, compile_evm_bytecode_legacy, compile_push1_program_legacy},
    execution::random_u256,
    rv64::{
        executor::{ExecutionReport, IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{
            initialize_guest_state, read_guest_result, write_evm_word, GuestExecutionStatus,
            GuestMetadata, GuestResult, GUEST_METADATA_ENCODED_SIZE,
        },
        memory::{
            LayoutError, EVM_STACK_CAPACITY, EVM_STACK_REGION, EVM_WORD_SIZE, METADATA_REGION,
        },
        verify::{verify_declared_transfers, DeclaredTransfer, Transfer},
    },
    DecodeError,
};

fn bytecode(immediates: &[u8], stop: bool) -> Vec<u8> {
    immediates
        .iter()
        .flat_map(|&value| [0x60, value])
        .chain(stop.then_some(0))
        .collect()
}

fn assert_layout(candidate: &CompiledRv64Program, pushes: usize, stop: bool) {
    let normal = ENTRY_PC + 44 * pushes as u64 + 4 * u64::from(stop);
    let has_overflow = pushes > 0;
    let expected_publishers = if has_overflow {
        vec![normal, normal + 28]
    } else {
        vec![normal]
    };
    assert_eq!(candidate.termination_entry_pcs(), expected_publishers);
    assert_eq!(
        candidate.machine_code().len(),
        44 * pushes + 4 * usize::from(stop) + 28 + 32 * usize::from(has_overflow)
    );
    assert_eq!(candidate.normal_termination_instructions(), 7);
    assert_eq!(
        candidate.exceptional_termination_instructions(),
        8 * u64::from(has_overflow)
    );
    let round_trip = assemble_image(candidate.assembly()).unwrap();
    assert_eq!(round_trip.base_address(), candidate.entry_pc());
    assert_eq!(round_trip.bytes(), candidate.machine_code());
    assert_eq!(
        round_trip.labels().len(),
        pushes * 3 + usize::from(stop) + 1 + usize::from(has_overflow)
    );
    println!(
        "pushes={pushes} stop={stop} source_bytes={} phase22_source_bytes={} labels={} image_bytes={} publishers={:x?}",
        candidate.assembly().len(),
        candidate.assembly().len()
            + usize::from(!has_overflow) * super::Publisher::Overflow.assembly().len(),
        round_trip.labels().len(),
        candidate.machine_code().len(),
        candidate.termination_entry_pcs()
    );
}

fn assert_legacy_layout(
    candidate: &CompiledRv64Program,
    legacy: &CompiledRv64Program,
    pushes: usize,
    stop: bool,
) {
    let legacy_size = legacy.machine_code().len();
    let unreachable_tail = 32 * (usize::from(stop) + usize::from(pushes == 0));
    assert_eq!(
        candidate.machine_code(),
        &legacy.machine_code()[..legacy_size - unreachable_tail]
    );
    assert_eq!(
        candidate.termination_entry_pcs(),
        &legacy.termination_entry_pcs()[..candidate.termination_entry_pcs().len()]
    );
    assert_eq!(
        candidate.normal_termination_instructions(),
        legacy.normal_termination_instructions()
    );
    assert_eq!(
        candidate.exceptional_termination_instructions(),
        if pushes == 0 {
            0
        } else {
            legacy.exceptional_termination_instructions()
        }
    );
}

fn make_fragments(immediates: &[u8], stop: bool) -> Vec<SymbolicFragment> {
    immediates
        .iter()
        .copied()
        .enumerate()
        .map(|(index, immediate)| generate_symbolic_push1(immediate, index))
        .chain(stop.then(stop_fragment))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    body: ExecutionReport,
    termination: ExecutionReport,
    trace: Vec<[u8; GUEST_METADATA_ENCODED_SIZE]>,
    guest: GuestResult,
}

fn observe(
    program: &CompiledRv64Program,
    initial: &[U256],
    pushes: usize,
    failing_push: Option<usize>,
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
    executor.write_register(
        IntegerRegister::EVM_STACK_NEXT_FREE,
        base + initial.len() as u64 * EVM_WORD_SIZE,
    );
    let sp = IntegerRegister::new(2).unwrap();
    executor.write_register(sp, 0x1234_5670);
    let guard = [0xa5; 32];
    executor
        .write_memory(EVM_STACK_REGION.end(), &guard)
        .unwrap();

    let publishers = program.termination_entry_pcs();
    let mut stops = publishers.to_vec();
    stops.push(program.exit_pc());
    stops.extend((0..pushes as u64).map(|i| ENTRY_PC + 44 * i + 40));
    let body = if let Some(index) = failing_push {
        let start = ENTRY_PC + 44 * index as u64;
        stops.push(start + 12);
        let check = executor
            .execute_until(ENTRY_PC, &stops, program.instruction_limit())
            .unwrap();
        assert_eq!(
            check,
            ExecutionReport {
                exit_pc: start + 40,
                executed_instructions: 10 * index as u64 + 3
            }
        );
        let jump = executor
            .execute_until(check.exit_pc, publishers, 1)
            .unwrap();
        assert_eq!(
            jump,
            ExecutionReport {
                exit_pc: publishers[1],
                executed_instructions: 1
            }
        );
        ExecutionReport {
            exit_pc: jump.exit_pc,
            executed_instructions: check.executed_instructions + jump.executed_instructions,
        }
    } else {
        let report = executor
            .execute_until(ENTRY_PC, &stops, program.instruction_limit())
            .unwrap();
        assert_eq!(report.exit_pc, publishers[0]);
        report
    };

    let words = if failing_push.is_some() { 8 } else { 7 };
    let (termination, trace) = observe_publication(&mut executor, program, body.exit_pc, words);
    let guest = read_guest_result(&mut executor, &termination).unwrap();
    assert_eq!(executor.read_register(sp), 0x1234_5670);
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        base
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        guest.final_s1()
    );
    let mut actual_guard = [0; 32];
    executor
        .read_memory(EVM_STACK_REGION.end(), &mut actual_guard)
        .unwrap();
    assert_eq!(actual_guard, guard);
    Observation {
        body,
        termination,
        trace,
        guest,
    }
}

fn observe_publication(
    executor: &mut RvemuExecutor,
    program: &CompiledRv64Program,
    entry: u64,
    words: u64,
) -> (ExecutionReport, Vec<[u8; GUEST_METADATA_ENCODED_SIZE]>) {
    // Check store words even when writing the initialized value makes a
    // store invisible in the byte snapshots (e.g. normal exception code 0).
    let stores = if words == 7 {
        [(8, 0x0093_3823), (12, 0x0003_3423), (20, 0x0053_3023)]
    } else {
        [(8, 0x0093_3823), (16, 0x0053_3423), (24, 0x0053_3023)]
    };
    let entry_offset = (entry - program.entry_pc()) as usize;
    for (offset, expected) in stores {
        let start = entry_offset + offset;
        let word = u32::from_le_bytes(program.machine_code()[start..start + 4].try_into().unwrap());
        assert_eq!(
            word,
            expected,
            "publisher store at {:#x}",
            entry + offset as u64
        );
    }
    let read_metadata = |executor: &mut RvemuExecutor| {
        let mut bytes = [0; GUEST_METADATA_ENCODED_SIZE];
        executor
            .read_memory(METADATA_REGION.start(), &mut bytes)
            .unwrap();
        bytes
    };
    let mut trace = vec![read_metadata(executor)];
    let mut pc = entry;
    for index in 1..=words {
        let next = if index == words {
            program.exit_pc()
        } else {
            pc + 4
        };
        let step = executor.execute(pc, next, 1).unwrap();
        assert_eq!(
            step,
            ExecutionReport {
                exit_pc: next,
                executed_instructions: 1
            }
        );
        trace.push(read_metadata(executor));
        pc = step.exit_pc;
    }
    (
        ExecutionReport {
            exit_pc: pc,
            executed_instructions: words,
        },
        trace,
    )
}

fn assert_result(observed: &Observation, expected_stack: &[U256], overflow: bool, body_words: u64) {
    let code = u64::from(overflow);
    assert_guest_result(observed, expected_stack, code, body_words);
}

fn assert_guest_result(
    observed: &Observation,
    expected_stack: &[U256],
    code: u64,
    body_words: u64,
) {
    use GuestExecutionStatus::{GuestException, Normal, Running};
    let base = EVM_STACK_REGION.start();
    let s1 = base + EVM_WORD_SIZE * expected_stack.len() as u64;
    let (status, publisher_words) = if code != 0 {
        (GuestException, 8)
    } else {
        (Normal, 7)
    };
    assert_eq!(observed.guest.status(), status);
    assert_eq!(observed.guest.exception_code(), code);
    assert_eq!(observed.guest.stack(), expected_stack);
    assert_eq!(observed.guest.final_s1(), s1);
    assert_eq!(observed.body.executed_instructions, body_words);
    assert_eq!(observed.termination.executed_instructions, publisher_words);

    // Initial metadata comes from guest initialization. The wire bytes change
    // after the pointer, exception-code and status stores at their ABI steps.
    let mut trace = vec![GuestMetadata::new(Running, 0, base).encode(); 3];
    trace.resize(
        if code != 0 { 5 } else { 6 },
        GuestMetadata::new(Running, 0, s1).encode(),
    );
    if code != 0 {
        trace.extend([GuestMetadata::new(Running, code, s1).encode(); 2]);
    }
    trace.extend([GuestMetadata::new(status, code, s1).encode(); 2]);
    assert_eq!(observed.trace, trace);
    println!("status={status:?} code={code} depth={} s1={s1:#x} publisher={:#x} RV64={body_words}+{publisher_words}={} metadata={:02x?}", expected_stack.len(), observed.body.exit_pc, body_words + publisher_words, observed.trace);
}

fn full_stack() -> Vec<U256> {
    let mut seed = 42;
    (0..EVM_STACK_CAPACITY)
        .map(|_| random_u256(&mut seed))
        .collect()
}

#[test]
fn public_publisher_inventory_counts_resident_exceptional_bodies() {
    let cases = [
        (&[][..], 0u64),
        (&[0x00][..], 0),
        (&[0x60, 1][..], 8),
        (&[0x50][..], 8),
        (&[0x60, 1, 0x50][..], 16),
    ];
    for (bytecode, expected) in cases {
        let compiled = compile_evm_bytecode(bytecode).unwrap();
        assert_eq!(
            compiled.exceptional_termination_instructions(),
            expected,
            "input={bytecode:02x?}"
        );
    }
}

#[test]
fn short_artifacts_and_execution_match_legacy() {
    for immediates in [&[][..], &[2], &[2, 3], &[0, 127, 128, 255, 96]] {
        for stop in [false, true] {
            let input = bytecode(immediates, stop);
            let fragments = make_fragments(immediates, stop);
            let candidate = compile(&input).unwrap();
            assert_eq!(candidate, compose(&fragments).unwrap());
            let legacy = compile_evm_bytecode_legacy(&input).unwrap();
            if !stop {
                assert_eq!(legacy, compile_push1_program_legacy(immediates).unwrap());
            }
            let pushes = immediates.len();
            assert_layout(&candidate, pushes, stop);
            assert_legacy_layout(&candidate, &legacy, pushes, stop);
            let normal = observe(&candidate, &[], pushes, None);
            assert_eq!(normal, observe(&legacy, &[], pushes, None));
            let expected: Vec<_> = immediates.iter().copied().map(U256::from).collect();
            assert_result(
                &normal,
                &expected,
                false,
                10 * pushes as u64 + u64::from(stop),
            );
        }
    }
}

#[test]
fn frontend_preserves_truncated_push_and_first_stop() {
    for (input, canonical, value, stop) in [
        (&[0x60][..], &[0x60, 0][..], 0, false),
        (&[0x60, 0xff, 0, 0xfe, 0x60], &[0x60, 0xff, 0], 255, true),
    ] {
        let candidate = compile(input).unwrap();
        assert_eq!(candidate, compile(canonical).unwrap());
        let legacy = compile_evm_bytecode_legacy(input).unwrap();
        assert_eq!(legacy, compile_evm_bytecode_legacy(canonical).unwrap());
        assert_legacy_layout(&candidate, &legacy, 1, stop);
        let actual = observe(&candidate, &[], 1, None);
        assert_eq!(actual, observe(&legacy, &[], 1, None));
        assert_result(
            &actual,
            &[U256::from(value as u8)],
            false,
            10 + u64::from(stop),
        );
    }
    assert_eq!(compile(&[0, 0xfe, 0x60]), compile(&[0]));
    assert_eq!(
        compile(&[0x60, 0, 0x61, 2]),
        Err(CompileError::UnsupportedInstruction {
            opcode: 0x61,
            offset: 2
        })
    );
    assert_eq!(
        compile(&[0x60, 0, 0xfe]),
        Err(CompileError::Decode(DecodeError::UnsupportedOpcode {
            opcode: 0xfe,
            offset: 2
        }))
    );
}

#[test]
fn long_layouts_match_phase_one_and_take_distant_overflow() {
    let full = full_stack();
    // Literal first JAL words, publisher PCs and counts from RF-013.
    for (pushes, stop, first_jal, displacement, normal_pc, overflow_pc, body_words) in [
        (46, false, 0x7dc0_006f, 2012, 0x8000_07e8, 0x8000_0804, 460),
        (46, true, 0x7e00_006f, 2016, 0x8000_07ec, 0x8000_0808, 461),
        (47, false, 0x0090_006f, 2056, 0x8000_0814, 0x8000_0830, 470),
        (47, true, 0x00d0_006f, 2060, 0x8000_0818, 0x8000_0834, 471),
    ] {
        let input = bytecode(&vec![2; pushes], stop);
        let candidate = compile(&input).unwrap();
        assert_layout(&candidate, pushes, stop);
        assert_eq!(candidate.termination_entry_pcs(), [normal_pc, overflow_pc]);
        assert_eq!(
            u32::from_le_bytes(candidate.machine_code()[40..44].try_into().unwrap()),
            first_jal
        );
        let normal = observe(&candidate, &[], pushes, None);
        assert_result(&normal, &vec![U256::from(2); pushes], false, body_words);
        let exceptional = observe(&candidate, &full, pushes, Some(0));
        assert_result(&exceptional, &full, true, 4);
        match compile_evm_bytecode_legacy(&input) {
            Ok(legacy) if pushes == 46 => {
                assert_legacy_layout(&candidate, &legacy, pushes, stop);
                assert_eq!(normal, observe(&legacy, &[], pushes, None));
                assert_eq!(exceptional, observe(&legacy, &full, pushes, Some(0)));
            }
            Err(CompileError::Link { description }) if pushes == 47 => {
                assert!(description.contains("ImmediateOverflow"), "{description}");
                assert!(
                    description.contains(&format!("value: {displacement}, min: -2048, max: 2047")),
                    "{description}"
                );
                println!("legacy pushes={pushes} stop={stop}: {description}");
            }
            other => {
                panic!("unexpected legacy compilation for {pushes} PUSH1, STOP={stop}: {other:?}")
            }
        }
    }
}

#[test]
fn push_from_one_below_capacity_before_next_push_fails() {
    let mut expected = full_stack();
    expected.pop();
    let initial = expected.clone();
    expected.push(U256::from(255));
    let input = [0x60, 255, 0x60, 1, 0];
    let candidate = compile(&input).unwrap();
    let legacy = compile_evm_bytecode_legacy(&input).unwrap();
    let actual = observe(&candidate, &initial, 2, Some(1));
    assert_eq!(actual, observe(&legacy, &initial, 2, Some(1)));
    assert_result(&actual, &expected, true, 14);
}

#[test]
fn final_image_rejects_missing_misdirected_and_expanded_transfers() {
    let fragments = make_fragments(&[2], true);
    let publishers = publisher_inventory(&fragments);
    let source = emit(&fragments, &publishers);
    for (from, to, diagnostic) in [
        (
            "jal zero, push1_stack_overflow_exit",
            "jal zero, normal_exit",
            "targets",
        ),
        (
            "jal zero, push1_continue_0",
            "addi zero, zero, 0",
            "unexpected control transfer",
        ),
        (
            "jal zero, push1_continue_0",
            "jal zero, push1_overflow_0",
            "targets",
        ),
        (
            "jal zero, normal_exit",
            "jal zero, push1_stack_overflow_exit",
            "targets",
        ),
        (
            "bgeu s1, t0, push1_overflow_0",
            "bltu s1, t0, 8\n    jal zero, push1_overflow_0",
            "expected",
        ),
        (
            "push1_stack_overflow_exit",
            "other_label",
            "missing publisher label",
        ),
    ] {
        let modified = source.replace(from, to);
        let result = assemble_image(&modified).unwrap();
        let err = verified_program(&fragments, &publishers, modified, result).unwrap_err();
        assert!(
            matches!(&err, CompileError::Link { description } if description.contains(diagnostic)),
            "fixture {from:?}: {err:?}"
        );
    }
    let image = assemble_image(&source).unwrap();
    assert!(verified_program(&fragments[..1], &publishers, source, image).is_err());
    assert!(compose(&[stop_fragment(), generate_symbolic_push1(2, 0)]).is_err());
}

#[test]
fn declarations_and_namespaces_reject_incomplete_or_corrupted_images() {
    let mut fragments = make_fragments(&[2], false);
    let publishers = publisher_inventory(&fragments);
    let source = emit(&fragments, &publishers);

    fragments[0].external_references.clear();
    let image = assemble_image(&source).unwrap();
    let missing = verified_program(&fragments, &publishers, source.clone(), image).unwrap_err();
    assert!(matches!(
        missing,
        CompileError::Link { description } if description.contains("unexpected control transfer")
    ));

    fragments = make_fragments(&[2], false);
    let external = fragments[0].external_references[0];
    fragments[0].external_references.push(external);
    let image = assemble_image(&source).unwrap();
    let duplicate = verified_program(&fragments, &publishers, source.clone(), image).unwrap_err();
    assert!(matches!(
        duplicate,
        CompileError::Link { description } if description.contains("duplicate symbolic transfer")
    ));

    fragments = make_fragments(&[2], false);
    fragments[0].external_references[0].role = PublisherRole::Normal;
    let image = assemble_image(&source).unwrap();
    let wrong_role = verified_program(&fragments, &publishers, source.clone(), image).unwrap_err();
    assert!(matches!(
        wrong_role,
        CompileError::Link { description } if description.contains("targets")
    ));

    for extra_transfer in [
        "    jal zero, normal_exit\n",
        "    beq zero, zero, normal_exit\n",
    ] {
        let expanded = source.replace(
            "push1_continue_0:\n",
            &format!("push1_continue_0:\n{extra_transfer}"),
        );
        let image = assemble_image(&expanded).unwrap();
        let error = verified_program(&make_fragments(&[2], false), &publishers, expanded, image)
            .unwrap_err();
        assert!(matches!(
            error,
            CompileError::Link { description } if description.contains("unexpected control transfer")
        ));
    }

    let corrupted = source.replace("jal zero, push1_continue_0", "jal zero, push1_overflow_0");
    let image = assemble_image(&corrupted).unwrap();
    let error =
        verified_program(&make_fragments(&[2], false), &publishers, corrupted, image).unwrap_err();
    assert!(matches!(
        error,
        CompileError::Link { description } if description.contains("targets")
    ));

    let collision =
        compose(&[generate_symbolic_push1(2, 0), generate_symbolic_push1(3, 0)]).unwrap_err();
    assert!(matches!(
        collision,
        CompileError::Link { description } if description.contains("duplicate fragment entry")
    ));
    let local_collision = source.replace("push1_overflow_0", "normal_exit");
    assert!(assemble_image(&local_collision).is_err());

    let image = assemble_image(&source).unwrap();
    let missing_publisher = verified_program(
        &make_fragments(&[2], false),
        &[PublisherRole::Normal],
        source,
        image,
    )
    .unwrap_err();
    assert!(matches!(
        missing_publisher,
        CompileError::Link { description } if description.contains("missing Overflow publisher")
    ));
}

mod arithmetic;
mod pop_dup1;

#[test]
fn pop_dup1_inventory_and_mixed_runtime() {
    for (input, roles) in [
        (
            &[0x50][..],
            vec![PublisherRole::Normal, PublisherRole::Underflow],
        ),
        (
            &[0x80][..],
            vec![
                PublisherRole::Normal,
                PublisherRole::Underflow,
                PublisherRole::Overflow,
            ],
        ),
        (
            &[0x60, 2, 0x50][..],
            vec![
                PublisherRole::Normal,
                PublisherRole::Underflow,
                PublisherRole::Overflow,
            ],
        ),
    ] {
        let instructions = decode_detailed(input).unwrap();
        let mut fragments = Vec::new();
        for (index, instruction) in instructions.iter().enumerate() {
            let continuation = if index + 1 < instructions.len() {
                match &instructions[index + 1] {
                    DecodedInstruction::Push { opcode: 0x60, .. } => {
                        format!("guest_push1_{}", index + 1)
                    }
                    DecodedInstruction::Pop { .. } => format!("guest_pop_{}", index + 1),
                    DecodedInstruction::Dup1 { .. } => format!("guest_dup1_{}", index + 1),
                    DecodedInstruction::Stop { .. } => "guest_stop".to_owned(),
                    _ => unreachable!(),
                }
            } else {
                PublisherRole::Normal.label().to_owned()
            };
            match instruction {
                DecodedInstruction::Push { value, .. } => {
                    fragments.push(generate_symbolic_push1(value.to::<u8>(), index))
                }
                DecodedInstruction::Pop { .. } => {
                    fragments.push(generate_symbolic_pop(index, continuation))
                }
                DecodedInstruction::Dup1 { .. } => {
                    fragments.push(generate_symbolic_dup1(index, continuation))
                }
                DecodedInstruction::Stop { .. } => fragments.push(stop_fragment()),
                _ => unreachable!(),
            }
        }
        let candidate = compose(&fragments).unwrap();
        assert_eq!(publisher_inventory(&fragments), roles);
        let round_trip = assemble_image(candidate.assembly()).unwrap();
        assert_eq!(round_trip.bytes(), candidate.machine_code());
    }

    let pop = compose(&[generate_symbolic_pop(
        0,
        PublisherRole::Normal.label().to_owned(),
    )])
    .unwrap();
    let pop_result = observe(&pop, &[U256::from(2)], 0, None);
    assert_result(&pop_result, &[], false, 3);

    let dup = compose(&[generate_symbolic_dup1(
        0,
        PublisherRole::Normal.label().to_owned(),
    )])
    .unwrap();
    let dup_result = observe(&dup, &[U256::from(2)], 0, None);
    assert_result(&dup_result, &[U256::from(2), U256::from(2)], false, 14);

    let mixed = compose(&[
        generate_symbolic_dup1(0, "guest_pop_1".to_owned()),
        generate_symbolic_pop(1, PublisherRole::Normal.label().to_owned()),
    ])
    .unwrap();
    let mixed_result = observe(&mixed, &[U256::from(2)], 0, None);
    assert_result(&mixed_result, &[U256::from(2)], false, 17);
}

#[test]
fn declaration_verifier_rejects_misalignment_and_executable_bounds() {
    let bytes = 0x0080_006f_u32.to_le_bytes();
    let declaration = [DeclaredTransfer {
        site_pc: ENTRY_PC + 2,
        target_pc: ENTRY_PC + 4,
        kind: Transfer::Jal { rd: 0 },
    }];
    let error = verify_declared_transfers(&bytes, ENTRY_PC + 4, &declaration).unwrap_err();
    assert!(error.contains("not four-byte aligned"), "{error}");

    let error = verify_declared_transfers(
        &bytes,
        ENTRY_PC + 8,
        &[DeclaredTransfer {
            site_pc: ENTRY_PC,
            target_pc: ENTRY_PC + 4,
            kind: Transfer::Jal { rd: 0 },
        }],
    )
    .unwrap_err();
    assert!(error.contains("outside the executable image"), "{error}");
}

#[test]
#[ignore = "assembles multi-megabyte sources at the ABI image boundary"]
fn push1_capacity_boundary() {
    for (pushes, stop, size, accepted) in [
        (23_829, false, 1_048_536, true),
        (23_830, false, 1_048_580, false),
        (23_829, true, 1_048_540, true),
        (23_830, true, 1_048_584, false),
    ] {
        let fragments = make_fragments(&vec![2; pushes], stop);
        let publishers = publisher_inventory(&fragments);
        let source = emit(&fragments, &publishers);
        let image = assemble_image(&source).unwrap();
        assert_eq!(image.len(), size);
        assert_eq!(image.labels().len(), 3 * pushes + usize::from(stop) + 2);
        let source_bytes = source.len();
        let labels = image.labels().len();
        let result = verified_program(&fragments, &publishers, source, image);
        if accepted {
            assert_eq!(result.unwrap().machine_code().len(), size);
        } else {
            assert_eq!(
                result,
                Err(CompileError::Layout(LayoutError::CodeOverlapsExitSlot {
                    code_end: ENTRY_PC + size as u64
                }))
            );
        }
        println!("pushes={pushes} stop={stop} source_bytes={source_bytes} labels={labels} asm=ok image_bytes={size} layout_accepted={accepted}");
    }
}

#[test]
fn unsupported_push_diagnostics_follow_decoded_order() {
    for input in [&[0x61, 0, 1, 0x62][..], &[0x50, 0x61, 0, 1, 0x62]] {
        let expected = CompileError::UnsupportedInstruction {
            opcode: 0x61,
            offset: input.len() - 4,
        };
        assert_eq!(compile_evm_bytecode(input), Err(expected.clone()));
        assert_eq!(compile(input), Err(expected));
    }
}

#[test]
fn matching_pop_continuation_cannot_skip_the_next_fragment() {
    let fragments = [
        generate_symbolic_pop(0, "guest_stop".to_owned()),
        generate_symbolic_pop(1, "guest_stop".to_owned()),
        stop_fragment(),
    ];
    let error = compose(&fragments).unwrap_err();
    assert!(
        matches!(error, CompileError::Link { ref description } if description.contains("targets 0x80000020, expected 0x80000010")),
        "{error}"
    );
}

#[test]
fn declaration_site_must_belong_to_its_fragment() {
    let mut fragments = make_fragments(&[2, 3], false);
    let mut stolen = fragments[1].external_references.remove(0);
    stolen.site_word += 11;
    fragments[0].external_references.push(stolen);
    let error = compose(&fragments).unwrap_err();
    assert!(
        matches!(error, CompileError::Link { description } if description.contains("outside its owning fragment"))
    );
}

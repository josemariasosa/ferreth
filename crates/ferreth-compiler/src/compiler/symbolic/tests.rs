use alloy_primitives::U256;

use super::*;
use crate::{
    compiler::{compile_evm_bytecode, compile_evm_bytecode_legacy, compile_push1_program_legacy},
    execution::random_u256,
    rv64::{
        executor::{ExecutionReport, IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{
            initialize_guest_state, read_guest_result, GuestExecutionStatus, GuestMetadata,
            GuestResult, GUEST_METADATA_ENCODED_SIZE,
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
    let normal = ENTRY_PC + 76 * pushes as u64 + 4 * u64::from(stop);
    let has_pushes = pushes > 0;
    let expected_publishers = if has_pushes {
        vec![normal, normal + 36, normal + 76]
    } else {
        vec![normal]
    };
    assert_eq!(candidate.termination_entry_pcs(), expected_publishers);
    assert_eq!(
        candidate.machine_code().len(),
        76 * pushes + 4 * usize::from(stop) + 36 + 80 * usize::from(has_pushes)
    );
    assert_eq!(candidate.normal_termination_instructions(), 9);
    assert_eq!(
        candidate.exceptional_termination_instructions(),
        20 * u64::from(has_pushes)
    );
    let round_trip = assemble_image(candidate.assembly()).unwrap();
    assert_eq!(round_trip.base_address(), candidate.entry_pc());
    assert_eq!(round_trip.bytes(), candidate.machine_code());
    assert_eq!(
        round_trip.labels().len(),
        pushes * 4 + usize::from(stop) + 1 + 2 * usize::from(has_pushes)
    );
}

fn assert_legacy_layout(
    candidate: &CompiledRv64Program,
    legacy: &CompiledRv64Program,
    pushes: usize,
    stop: bool,
) {
    let legacy_body = 44 * pushes + 4 * usize::from(stop);
    let metered_body = legacy_body + 32 * pushes;
    let unreachable_tail = 40 * (usize::from(stop) + usize::from(pushes == 0));
    assert_eq!(
        legacy.machine_code().len() - unreachable_tail,
        legacy_body + 36 + 40 * usize::from(pushes > 0)
    );
    for index in 0..pushes {
        // All ten preexisting body words stay identical. The external
        // overflow JAL relocates because subsequent fragments grow.
        let old = 44 * index;
        let new = 76 * index + 32;
        assert_eq!(
            candidate.machine_code()[new..new + 40],
            legacy.machine_code()[old..old + 40]
        );
    }
    if stop {
        assert_eq!(
            candidate.machine_code()[metered_body - 4..metered_body],
            legacy.machine_code()[legacy_body - 4..legacy_body]
        );
    }
    let common_tail = 36 + 40 * usize::from(pushes > 0);
    assert_eq!(
        candidate.machine_code()[metered_body..metered_body + common_tail],
        legacy.machine_code()[legacy_body..legacy_body + common_tail]
    );
    for (index, offset) in [0, 36]
        .into_iter()
        .take(1 + usize::from(pushes > 0))
        .enumerate()
    {
        assert_eq!(
            legacy.termination_entry_pcs()[index],
            ENTRY_PC + (legacy_body + offset) as u64
        );
        assert_eq!(
            candidate.termination_entry_pcs()[index],
            ENTRY_PC + (metered_body + offset) as u64
        );
    }
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
    metered: bool,
) -> Observation {
    const LEGACY_OVERFLOW: &[(u64, u32)] = &[(0, 0x37), (4, 0x33), (8, 0x63), (40, 0x6f)];
    let path = if metered {
        METERED_OVERFLOW
    } else {
        LEGACY_OVERFLOW
    };
    let width = if metered { 76 } else { 44 };
    let fault = failing_push.map(|index| (width * index as u64, path));
    let limit = if metered {
        3 * failing_push.map_or(pushes, |index| index + 1) as u64
    } else {
        0
    };
    let code = u64::from(fault.is_some());
    pop_dup1::observe_guest(
        program,
        initial,
        program.termination_entry_pcs()[code as usize],
        code,
        fault,
        &[],
        limit,
    )
}

const METERED_OVERFLOW: &[(u64, u32)] = &[
    (0, 0x13),
    (4, 0x13),
    (8, 0x03),
    (12, 0x13),
    (16, 0x63),
    (24, 0x33),
    (28, 0x23),
    (32, 0x37),
    (36, 0x33),
    (40, 0x63),
    (72, 0x6f),
];

// Legacy word offsets let the replay fixtures keep their original scope.
// Only PUSH1 widens, by eight words, in the metered image.
fn legacy_push_offsets(input: &[u8]) -> Vec<u64> {
    let mut at = 0;
    let mut pushes = Vec::new();
    for op in decode_detailed(input).unwrap() {
        at += match op {
            DecodedInstruction::Push { .. } => {
                pushes.push(at);
                44
            }
            DecodedInstruction::Stop { .. } => 4,
            DecodedInstruction::Pop { .. } => 16,
            DecodedInstruction::Dup1 { .. } => 64,
            DecodedInstruction::Add { .. } | DecodedInstruction::Sub { .. } => 172,
        };
    }
    pushes
}

fn observe_publication(
    executor: &mut RvemuExecutor,
    program: &CompiledRv64Program,
    entry: u64,
    words: u64,
) -> (ExecutionReport, Vec<[u8; GUEST_METADATA_ENCODED_SIZE]>) {
    // Check store words even when writing the initialized value makes a
    // store invisible in the byte snapshots (e.g. normal exception code 0).
    let stores = if words == 9 {
        [(8, 0x0093_3823), (12, 0x0003_3423), (28, 0x0053_3023)]
    } else {
        [(8, 0x0093_3823), (16, 0x0053_3423), (32, 0x0053_3023)]
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
    assert_guest_result(observed, expected_stack, u64::from(overflow), body_words);
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
        (GuestException, 10)
    } else {
        (Normal, 9)
    };
    assert_eq!(observed.guest.status(), status);
    assert_eq!(observed.guest.exception_code(), code);
    assert_eq!(observed.guest.stack(), expected_stack);
    assert_eq!(observed.guest.final_s1(), s1);
    let limit = observed.guest.gas_limit();
    assert_eq!(observed.guest.remaining_gas(), 0);
    assert_eq!(observed.guest.gas_used(), limit);
    assert_eq!(observed.body.executed_instructions, body_words);
    assert_eq!(observed.termination.executed_instructions, publisher_words);

    // Initial metadata comes from guest initialization. The wire bytes change
    // after the pointer, exception-code and status stores at their ABI steps.
    let mut trace = vec![GuestMetadata::new(Running, 0, base, limit, 0, 0).encode(); 3];
    trace.resize(
        if code != 0 { 5 } else { 8 },
        GuestMetadata::new(Running, 0, s1, limit, 0, 0).encode(),
    );
    if code != 0 {
        trace.extend([GuestMetadata::new(Running, code, s1, limit, 0, 0).encode(); 4]);
    }
    trace.extend([GuestMetadata::new(status, code, s1, limit, 0, 0).encode(); 2]);
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
        (&[0x60, 1][..], 20),
        (&[0x50][..], 10),
        (&[0x60, 1, 0x50][..], 30),
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
    for immediates in [&[][..], &[2], &[2, 3], &[5, 3], &[0, 127, 128, 255, 96]] {
        for stop in [false, true] {
            let input = bytecode(immediates, stop);
            let fragments = make_fragments(immediates, stop);
            let candidate = compile_evm_bytecode(&input).unwrap();
            assert_eq!(candidate, compose(&fragments).unwrap());
            let legacy = compile_evm_bytecode_legacy(&input).unwrap();
            if !stop {
                assert_eq!(legacy, compile_push1_program_legacy(immediates).unwrap());
            }
            let pushes = immediates.len();
            assert_layout(&candidate, pushes, stop);
            assert_legacy_layout(&candidate, &legacy, pushes, stop);
            let normal = observe(&candidate, &[], pushes, None, true);
            let reference = observe(&legacy, &[], pushes, None, false);
            let expected: Vec<_> = immediates.iter().copied().map(U256::from).collect();
            assert_result(
                &reference,
                &expected,
                false,
                10 * pushes as u64 + u64::from(stop),
            );
            assert_result(
                &normal,
                &expected,
                false,
                17 * pushes as u64 + u64::from(stop),
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
        let actual = observe(&candidate, &[], 1, None, true);
        let reference = observe(&legacy, &[], 1, None, false);
        assert_result(
            &reference,
            &[U256::from(value as u8)],
            false,
            10 + u64::from(stop),
        );
        assert_result(
            &actual,
            &[U256::from(value as u8)],
            false,
            17 + u64::from(stop),
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
    // Historical numeric JAL boundary stays fixed. Metered addresses follow
    // from 76 bytes per push and a 36-byte normal publisher.
    for (pushes, stop, displacement, metered_jal) in [
        (46, false, 2020, 0x5850_006f),
        (46, true, 2024, 0x5890_006f),
        (47, false, 2064, 0x5d10_006f),
        (47, true, 2068, 0x5d50_006f),
    ] {
        let input = bytecode(&vec![2; pushes], stop);
        let candidate = compile(&input).unwrap();
        assert_layout(&candidate, pushes, stop);
        assert_eq!(
            u32::from_le_bytes(candidate.machine_code()[72..76].try_into().unwrap()),
            metered_jal
        );
        let normal = observe(&candidate, &[], pushes, None, true);
        assert_result(
            &normal,
            &vec![U256::from(2); pushes],
            false,
            17 * pushes as u64 + u64::from(stop),
        );
        let exceptional = observe(&candidate, &full, pushes, Some(0), true);
        assert_result(&exceptional, &full, true, 11);
        match compile_evm_bytecode_legacy(&input) {
            Ok(legacy) if pushes == 46 => {
                assert_legacy_layout(&candidate, &legacy, pushes, stop);
                assert_result(
                    &observe(&legacy, &[], pushes, None, false),
                    &vec![U256::from(2); pushes],
                    false,
                    10 * pushes as u64 + u64::from(stop),
                );
                assert_result(
                    &observe(&legacy, &full, pushes, Some(0), false),
                    &full,
                    true,
                    4,
                );
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
    let actual = observe(&candidate, &initial, 2, Some(1), true);
    assert_result(
        &observe(&legacy, &initial, 2, Some(1), false),
        &expected,
        true,
        14,
    );
    assert_result(&actual, &expected, true, 28);
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
                PublisherRole::OutOfGas,
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
    let pop_result = observe(&pop, &[U256::from(2)], 0, None, false);
    assert_result(&pop_result, &[], false, 3);

    let dup = compose(&[generate_symbolic_dup1(
        0,
        PublisherRole::Normal.label().to_owned(),
    )])
    .unwrap();
    let dup_result = observe(&dup, &[U256::from(2)], 0, None, false);
    assert_result(&dup_result, &[U256::from(2), U256::from(2)], false, 14);

    let mixed = compose(&[
        generate_symbolic_dup1(0, "guest_pop_1".to_owned()),
        generate_symbolic_pop(1, PublisherRole::Normal.label().to_owned()),
    ])
    .unwrap();
    let mixed_result = observe(&mixed, &[U256::from(2)], 0, None, false);
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
        (13_795, false, 1_048_536, true),
        (13_796, false, 1_048_612, false),
        (13_795, true, 1_048_540, true),
        (13_796, true, 1_048_616, false),
    ] {
        let fragments = make_fragments(&vec![2; pushes], stop);
        let publishers = publisher_inventory(&fragments);
        let source = emit(&fragments, &publishers);
        let image = assemble_image(&source).unwrap();
        assert_eq!(image.len(), size);
        assert_eq!(image.labels().len(), 4 * pushes + usize::from(stop) + 3);
        let source_bytes = source.len();
        let labels = image.labels().len();
        let result = verified_program(&fragments, &publishers, source, image);
        if accepted {
            let program = result.unwrap();
            assert_eq!(program.machine_code().len(), size);
            let target = ENTRY_PC + 76 * pushes as u64 + 4 * u64::from(stop) + 76;
            assert_eq!(program.termination_entry_pcs()[2], target);
            let observed = pop_dup1::observe_guest(
                &program,
                &[],
                target,
                3,
                Some((0, OOG_CHARGE_PATH)),
                &[],
                0,
            );
            assert_guest_result(&observed, &[], 3, 6);
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
    // The first OOG JAL is at byte 20 and its publisher follows the
    // 76-byte normal/overflow tail prefix. One more PUSH exceeds J reach.
    for (stop, displacement) in [(false, 1_048_628), (true, 1_048_632)] {
        let fragments = make_fragments(&vec![2; 13_797], stop);
        let publishers = publisher_inventory(&fragments);
        let error = assemble_image(&emit(&fragments, &publishers)).unwrap_err();
        let diagnostic = format!(
            "BranchOutOfRange {{ label: \"out_of_gas_exit\", disp: {displacement}, max: 1048574"
        );
        assert!(error.to_string().contains(&diagnostic), "{error}");
        println!("pushes=13797 stop={stop} {error}");
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
fn matching_push1_continuation_cannot_skip_the_next_fragment() {
    let mut fragments = make_fragments(&[1, 2, 3], false);
    let original = "jal zero, push1_continue_0";
    assert_eq!(fragments[0].source.matches(original).count(), 1);
    fragments[0].source = fragments[0]
        .source
        .replace(original, "jal zero, guest_push1_2");
    let mut continuations = fragments[0]
        .local_transfers
        .iter_mut()
        .filter(|declaration| declaration.site_word == 17);
    continuations.next().unwrap().target_symbol = "guest_push1_2".to_owned();
    assert!(continuations.next().is_none());

    let publishers = publisher_inventory(&fragments);
    assert_eq!(
        publishers,
        [
            PublisherRole::Normal,
            PublisherRole::Overflow,
            PublisherRole::OutOfGas
        ]
    );
    let source = emit(&fragments, &publishers);
    let image = assemble_image(&source).expect("mutated source must assemble");
    let publisher_pcs: Vec<_> = publishers
        .iter()
        .map(|role| image.label_address(role.label()).unwrap())
        .collect();
    let normal_pc = publisher_pcs[0];
    let mut fragment_end = normal_pc;
    let mut declarations = Vec::new();
    for fragment in fragments.iter().rev() {
        let entry_pc = image.label_address(&fragment.entry_symbol).unwrap();
        for declaration in &fragment.local_transfers {
            declarations.push(DeclaredTransfer {
                site_pc: transfer_site_pc(entry_pc, fragment_end, declaration.site_word).unwrap(),
                target_pc: image.label_address(&declaration.target_symbol).unwrap(),
                kind: declaration.kind,
            });
        }
        for declaration in &fragment.external_references {
            declarations.push(DeclaredTransfer {
                site_pc: transfer_site_pc(entry_pc, fragment_end, declaration.site_word).unwrap(),
                target_pc: image.label_address(declaration.role.label()).unwrap(),
                kind: declaration.kind,
            });
        }
        fragment_end = entry_pc;
    }

    verify_declared_transfers(image.bytes(), normal_pc, &declarations)
        .expect("encoded transfers must agree with their declarations");
    let error = verify_final_image(
        image.bytes(),
        fragments.iter().map(|fragment| fragment.fixed_layout),
        PublisherLayout::SymbolicNormalOverflowOutOfGas,
        &publisher_pcs,
    )
    .unwrap_err();
    assert!(error.contains("targets"), "{error}");
    assert!(error.contains("0x80000098"), "{error}");
    assert!(error.contains("0x8000004c"), "{error}");
}

#[test]
fn declaration_site_must_belong_to_its_fragment() {
    let mut fragments = make_fragments(&[2, 3], false);
    let mut stolen = fragments[1].external_references.remove(0);
    stolen.site_word += 19;
    fragments[0].external_references.push(stolen);
    let error = compose(&fragments).unwrap_err();
    assert!(
        matches!(error, CompileError::Link { description } if description.contains("outside its owning fragment"))
    );
}

#[test]
fn normal_publisher_writes_remaining_before_status() {
    let program = compile_evm_bytecode(&[0]).unwrap();
    let entry = program.termination_entry_pcs()[0];
    for limit in [7, 0] {
        let mut executor = RvemuExecutor::new().unwrap();
        executor.load_code(program.machine_code()).unwrap();
        executor
            .write_memory(program.exit_pc(), &0x0010_0073_u32.to_le_bytes())
            .unwrap();
        initialize_guest_state(&mut executor, limit).unwrap();
        executor.execute(ENTRY_PC, entry, 1).unwrap();
        let mut initial = [0; 48];
        executor
            .read_memory(METADATA_REGION.start(), &mut initial)
            .unwrap();
        let record = |status, published| {
            GuestMetadata::new(status, 0, EVM_STACK_REGION.start(), limit, limit, published)
                .encode()
        };
        assert_eq!(initial, record(GuestExecutionStatus::Running, 0));
        executor
            .write_memory(METADATA_REGION.start() + 40, &u64::MAX.to_le_bytes())
            .unwrap();
        let (exit, trace) = observe_publication(
            &mut executor,
            &program,
            entry,
            program.normal_termination_instructions(),
        );
        assert_eq!(trace[0], record(GuestExecutionStatus::Running, u64::MAX));
        assert_eq!(
            &trace[0..6],
            &[record(GuestExecutionStatus::Running, u64::MAX); 6]
        );
        assert_eq!(
            &trace[6..8],
            &[record(GuestExecutionStatus::Running, limit); 2]
        );
        assert_eq!(
            trace[trace.len() - 2],
            record(GuestExecutionStatus::Normal, limit)
        );
        assert_eq!(exit.exit_pc, program.exit_pc());
        let guest = read_guest_result(&mut executor, &exit, limit).unwrap();
        assert_eq!(guest.remaining_gas(), limit);
    }
}

const OOG_CHARGE_PATH: &[(u64, u32)] = &[
    (0, 0x13),
    (4, 0x13),
    (8, 0x03),
    (12, 0x13),
    (16, 0x63),
    (20, 0x6f),
];

#[test]
fn metered_push_failure_preserves_allocation_and_settles_residual_gas() {
    use GuestExecutionStatus::{GuestException, Running};
    let full = full_stack();
    let initial = &full[..full.len() - 1];
    let mut expected = initial.to_vec();
    expected.push(U256::from(255));
    let program = compile(&[0x60, 255, 0x60, 1, 0]).unwrap();
    let publishers = program.termination_entry_pcs();
    for (limit, code, publisher, residual, path) in [
        (5, 3, publishers[2], 2, OOG_CHARGE_PATH),
        (11, 1, publishers[1], 5, METERED_OVERFLOW),
    ] {
        let observed = pop_dup1::observe_guest(
            &program,
            initial,
            publisher,
            code,
            Some((76, path)),
            &[],
            limit,
        );
        let full_s1 = EVM_STACK_REGION.end();
        let record = |status, code, pointer, live| {
            GuestMetadata::new(status, code, pointer, limit, live, 0).encode()
        };
        assert_eq!(observed.body.executed_instructions, 17 + path.len() as u64);
        assert_eq!(observed.guest.status(), GuestException);
        assert_eq!(observed.guest.exception_code(), code);
        assert_eq!(observed.guest.stack(), expected);
        assert_eq!(observed.guest.final_s1(), full_s1);
        assert_eq!(observed.guest.remaining_gas(), 0);
        assert_eq!(observed.guest.gas_used(), limit);
        assert_eq!(
            observed.trace[0],
            record(Running, 0, EVM_STACK_REGION.start(), residual)
        );
        assert_eq!(observed.trace[5], record(Running, code, full_s1, residual));
        assert_eq!(observed.trace[6..9], [record(Running, code, full_s1, 0); 3]);
        assert_eq!(
            observed.trace[9..],
            [record(GuestException, code, full_s1, 0); 2]
        );
    }
}

#[test]
fn first_push_reaches_oog_beyond_conditional_branch_range() {
    let program = compile(&bytecode(&[2; 55], false)).unwrap();
    let target = program.termination_entry_pcs()[2];
    assert_eq!(target, ENTRY_PC + 55 * 76 + 76);
    assert!(target - (ENTRY_PC + 16) > 4094);
    let observed =
        pop_dup1::observe_guest(&program, &[], target, 3, Some((0, OOG_CHARGE_PATH)), &[], 0);
    assert_guest_result(&observed, &[], 3, 6);
}

#[test]
fn final_image_rejects_diverted_oog_even_when_declaration_agrees() {
    let mut fragments = make_fragments(&[2], false);
    let publishers = publisher_inventory(&fragments);
    let source = emit(&fragments, &publishers).replace(
        "jal zero, out_of_gas_exit",
        "jal zero, push1_stack_overflow_exit",
    );
    let image = assemble_image(&source).unwrap();
    let error = verified_program(&fragments, &publishers, source.clone(), image).unwrap_err();
    assert!(error.to_string().contains("targets"));

    let declaration = fragments[0]
        .external_references
        .iter_mut()
        .find(|declaration| declaration.role == PublisherRole::OutOfGas)
        .unwrap();
    declaration.role = PublisherRole::Overflow;
    let image = assemble_image(&source).unwrap();
    let error = verified_program(&fragments, &publishers, source, image).unwrap_err();
    assert!(error
        .to_string()
        .contains("targets 0x80000070, expected 0x80000098"));
}

#[test]
fn final_image_rejects_corrupted_charge_and_oog_publication() {
    let fragments = make_fragments(&[2], false);
    let publishers = publisher_inventory(&fragments);
    let source = emit(&fragments, &publishers);
    let (body, oog) = source.split_once("out_of_gas_exit:\n").unwrap();
    for (mutated, diagnostic) in
        [
            (
                source.replace("bgeu t0, t2", "bge t0, t2"),
                "expected Branch",
            ),
            (
                source.replace("jal zero, out_of_gas_exit", "addi zero, zero, 0"),
                "unexpected control transfer",
            ),
            (
                source.replace("sub t0, t0, t2", "add t0, t0, t2"),
                "expected 0x407282b3",
            ),
            (
                source.replace("sd t0, 32(t1)", "addi zero, zero, 0"),
                "expected 0x02533023",
            ),
            (
                format!(
                    "{body}out_of_gas_exit:\n{}",
                    oog.replace("sd zero, 40(t1)", "addi zero, zero, 0")
                ),
                "OutOfGas publisher",
            ),
            (
                format!("{body}out_of_gas_exit:\n{}", oog.replace(
                "sd zero, 32(t1)\n    sd zero, 40(t1)\n    addi t0, zero, 2\n    sd t0, 0(t1)",
                "addi t0, zero, 2\n    sd t0, 0(t1)\n    sd zero, 32(t1)\n    sd zero, 40(t1)"
            )),
                "OutOfGas publisher",
            ),
        ]
    {
        assert_ne!(mutated, source);
        let image = assemble_image(&mutated).unwrap();
        let error = verified_program(&fragments, &publishers, mutated, image).unwrap_err();
        assert!(error.to_string().contains(diagnostic), "{error}");
    }
}

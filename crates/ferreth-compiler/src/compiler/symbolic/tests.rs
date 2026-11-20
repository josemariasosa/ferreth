use alloy_primitives::U256;

use super::*;
use crate::{
    compiler::{compile_evm_bytecode, compile_push1_program},
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
    assert_eq!(candidate.termination_entry_pcs(), [normal, normal + 28]);
    assert_eq!(
        candidate.machine_code().len(),
        44 * pushes + 4 * usize::from(stop) + 60
    );
    assert_eq!(candidate.normal_termination_instructions(), 7);
    assert_eq!(candidate.exceptional_termination_instructions(), 8);
    let round_trip = assemble_image(candidate.assembly()).unwrap();
    assert_eq!(round_trip.base_address(), candidate.entry_pc());
    assert_eq!(round_trip.bytes(), candidate.machine_code());
    assert_eq!(
        round_trip.labels().len(),
        pushes * 3 + usize::from(stop) + 2
    );
    println!(
        "pushes={pushes} stop={stop} bytes={} publishers={:x?}",
        candidate.machine_code().len(),
        candidate.termination_entry_pcs()
    );
}

fn assert_legacy_layout(candidate: &CompiledRv64Program, legacy: &CompiledRv64Program, stop: bool) {
    let legacy_size = legacy.machine_code().len();
    let unreachable_tail = 32 * usize::from(stop);
    assert_eq!(
        candidate.machine_code(),
        &legacy.machine_code()[..legacy_size - unreachable_tail]
    );
    assert_eq!(
        candidate.termination_entry_pcs(),
        &legacy.termination_entry_pcs()[..2]
    );
    assert_eq!(legacy.termination_entry_pcs().len(), 2 + usize::from(stop));
    assert_eq!(
        candidate.normal_termination_instructions(),
        legacy.normal_termination_instructions()
    );
    assert_eq!(
        candidate.exceptional_termination_instructions(),
        legacy.exceptional_termination_instructions()
    );
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

    let read_metadata = |executor: &mut RvemuExecutor| {
        let mut bytes = [0; GUEST_METADATA_ENCODED_SIZE];
        executor
            .read_memory(METADATA_REGION.start(), &mut bytes)
            .unwrap();
        bytes
    };
    let mut trace = vec![read_metadata(&mut executor)];
    let words = if failing_push.is_some() { 8 } else { 7 };
    let mut pc = body.exit_pc;
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
        trace.push(read_metadata(&mut executor));
        pc = step.exit_pc;
    }
    let termination = ExecutionReport {
        exit_pc: pc,
        executed_instructions: words,
    };
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

fn assert_result(observed: &Observation, expected_stack: &[U256], overflow: bool, body_words: u64) {
    use GuestExecutionStatus::{GuestException, Normal, Running};
    let base = EVM_STACK_REGION.start();
    let s1 = base + EVM_WORD_SIZE * expected_stack.len() as u64;
    let (status, code, publisher_words) = if overflow {
        (GuestException, 1, 8)
    } else {
        (Normal, 0, 7)
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
        if overflow { 5 } else { 6 },
        GuestMetadata::new(Running, 0, s1).encode(),
    );
    if overflow {
        trace.extend([GuestMetadata::new(Running, 1, s1).encode(); 2]);
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
fn short_artifacts_and_execution_match_legacy() {
    for immediates in [&[][..], &[2], &[2, 3], &[0, 127, 128, 255, 96]] {
        for stop in [false, true] {
            let input = bytecode(immediates, stop);
            let fragments: Vec<_> = immediates
                .iter()
                .copied()
                .map(SymbolicFragment::Push1)
                .chain(stop.then_some(SymbolicFragment::Stop))
                .collect();
            let candidate = compile(&input).unwrap();
            assert_eq!(candidate, compose(&fragments).unwrap());
            let legacy = compile_evm_bytecode(&input).unwrap();
            if !stop {
                assert_eq!(legacy, compile_push1_program(immediates).unwrap());
            }
            let pushes = immediates.len();
            assert_layout(&candidate, pushes, stop);
            assert_legacy_layout(&candidate, &legacy, stop);
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
        let legacy = compile_evm_bytecode(input).unwrap();
        assert_eq!(legacy, compile_evm_bytecode(canonical).unwrap());
        assert_legacy_layout(&candidate, &legacy, stop);
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
        compile(&[0x60, 0, 0x01]),
        Err(CompileError::UnsupportedInstruction {
            opcode: 0x01,
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
        match compile_evm_bytecode(&input) {
            Ok(legacy) if pushes == 46 => {
                assert_legacy_layout(&candidate, &legacy, stop);
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
    let legacy = compile_evm_bytecode(&input).unwrap();
    let actual = observe(&candidate, &initial, 2, Some(1));
    assert_eq!(actual, observe(&legacy, &initial, 2, Some(1)));
    assert_result(&actual, &expected, true, 14);
}

#[test]
fn final_image_rejects_missing_misdirected_and_expanded_transfers() {
    let fragments = [SymbolicFragment::Push1(2), SymbolicFragment::Stop];
    let source = emit(&fragments);
    for (from, to, diagnostic) in [
        (
            "jal zero, push1_stack_overflow_exit",
            "jal zero, normal_exit",
            "targets",
        ),
        (
            "jal zero, push1_continue_0",
            "addi zero, zero, 0",
            "unexpected opcode",
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
        let err = verified_program(&fragments, modified, result).unwrap_err();
        assert!(
            matches!(err, CompileError::Link { description } if description.contains(diagnostic))
        );
    }
    let image = assemble_image(&source).unwrap();
    assert!(verified_program(&fragments[..1], source, image).is_err());
    assert!(compose(&[SymbolicFragment::Stop, SymbolicFragment::Push1(2)]).is_err());
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
        let mut fragments = vec![SymbolicFragment::Push1(2); pushes];
        fragments.extend(stop.then_some(SymbolicFragment::Stop));
        let source = emit(&fragments);
        let image = assemble_image(&source).unwrap();
        assert_eq!(image.len(), size);
        assert_eq!(image.labels().len(), 3 * pushes + usize::from(stop) + 2);
        let source_bytes = source.len();
        let labels = image.labels().len();
        let result = verified_program(&fragments, source, image);
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

use asm_rs::RelocKind::{RvBranch12, RvJal20};
use ferreth_compiler::{
    compiler::PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE,
    execution::random_u256,
    rv64::{
        executor::{ExecutionReport, IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{
            initialize_guest_state, read_guest_result, write_evm_word, GuestExecutionStatus,
            GuestMetadata, GuestResult, GUEST_METADATA_ENCODED_SIZE,
        },
        memory::{EVM_STACK_CAPACITY, EVM_STACK_REGION, EVM_WORD_SIZE, METADATA_REGION},
    },
};

include!("support/symbolic_push1.rs");

#[test]
fn long_range_layouts_execute_normally() {
    // The first JAL fixtures are independently recorded by the Phase 0 probe.
    for (pushes, stop, size, normal, overflow, first_jal, body_count) in [
        (46, false, 2084, 0x8000_07e8, 0x8000_0804, 0x7dc0_006f, 460),
        (46, true, 2088, 0x8000_07ec, 0x8000_0808, 0x7e00_006f, 461),
        (47, false, 2128, 0x8000_0814, 0x8000_0830, 0x0090_006f, 470),
        (47, true, 2132, 0x8000_0818, 0x8000_0834, 0x00d0_006f, 471),
    ] {
        println!("Layout: pushes={pushes}, explicit_stop={stop}");
        let immediates = vec![2; pushes];
        let (_, artifact) = assemble_push_guest(&immediates, stop);
        verify_push_guest(&artifact, &immediates, stop);
        assert_eq!(artifact.len(), size);
        assert_eq!(artifact.label_address(NORMAL), Some(normal));
        assert_eq!(artifact.label_address(OVERFLOW), Some(overflow));
        assert_eq!(read_word(artifact.bytes(), ENTRY + 40), first_jal);
        println!(
            "{} bytes / {} RV64 words",
            artifact.len(),
            artifact.len() / 4
        );
        println!("Final labels: {:08x?}", artifact.labels());
        println!("Applied relocations: {:?}", artifact.relocations());

        let mut expected_relocations = Vec::new();
        let mut breakpoints = vec![normal, overflow, EXIT];
        for index in 0..pushes {
            let offset = 44 * index;
            expected_relocations.extend([
                (offset + 8, RvBranch12, format!("push1_overflow_{index}")),
                (offset + 36, RvJal20, format!("push1_continue_{index}")),
                (offset + 40, RvJal20, OVERFLOW.to_owned()),
            ]);
            breakpoints.push(ENTRY + offset as u64 + 40);
        }
        if stop {
            expected_relocations.push((44 * pushes, RvJal20, NORMAL.to_owned()));
        }
        let relocations = artifact.relocations();
        assert_eq!(relocations.len(), expected_relocations.len());
        for (actual, (offset, kind, label)) in relocations.iter().zip(expected_relocations) {
            assert_eq!(
                (actual.offset, actual.kind, &actual.label),
                (offset, kind, &label)
            );
            assert_eq!((actual.size, actual.addend), (4, 0));
        }

        let mut executor = load_guest(&artifact);
        // Every exception trampoline is a pre-fetch stop, so reaching normal
        // proves none of them was visited, even if a wrong jump could recover.
        let body = executor
            .execute_until(ENTRY, &breakpoints, artifact.len() as u64 / 4)
            .unwrap();
        assert_eq!(
            body,
            ExecutionReport {
                executed_instructions: body_count,
                exit_pc: normal
            }
        );

        let stack_base = EVM_STACK_REGION.start();
        assert_eq!(
            read_metadata(&mut executor),
            GuestMetadata::new(GuestExecutionStatus::Running, 0, stack_base, 0, 0, 0)
        );
        let final_s1 = stack_base + pushes as u64 * EVM_WORD_SIZE;
        let guest = finish_guest(
            &mut executor,
            body,
            GuestMetadata::new(GuestExecutionStatus::Normal, 0, final_s1, 0, 0, 0),
            7,
        );
        assert_eq!(guest.stack_depth(), pushes as u64);
        assert_eq!(guest.stack().len(), pushes);
        assert!(guest
            .stack()
            .iter()
            .all(|word| word.as_limbs() == &[2, 0, 0, 0]));
    }
}

#[test]
fn first_push_takes_the_distant_overflow_jump() {
    const TRAMPOLINE: u64 = ENTRY + 40;
    let mut seed = 42;
    let initial_stack: Vec<_> = (0..EVM_STACK_CAPACITY)
        .map(|_| random_u256(&mut seed))
        .collect();
    let full_s1 = EVM_STACK_REGION.end();
    let guard = [0xa5; 32];

    for (stop, normal, overflow) in [
        (false, 0x8000_0814, 0x8000_0830),
        (true, 0x8000_0818, 0x8000_0834),
    ] {
        println!("Full stack: pushes=47, explicit_stop={stop}");
        let (_, artifact) = assemble_push_guest(&[2; 47], stop);
        verify_jal(artifact.bytes(), TRAMPOLINE, overflow);
        let mut executor = load_guest(&artifact);
        for (slot, &word) in initial_stack.iter().enumerate() {
            write_evm_word(&mut executor, slot, word).unwrap();
        }
        executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, full_s1);
        executor.write_memory(full_s1, &guard).unwrap();
        let running = GuestMetadata::new(GuestExecutionStatus::Running, 0, full_s1, 0, 0, 0);
        executor
            .write_memory(METADATA_REGION.start(), &running.encode())
            .unwrap();

        // Stop before the first store path or trampoline. The three executed
        // words are the limit setup and the unsigned overflow branch.
        let check = executor
            .execute_until(ENTRY, &[ENTRY + 12, TRAMPOLINE], 3)
            .unwrap();
        assert_eq!(
            check,
            ExecutionReport {
                executed_instructions: 3,
                exit_pc: TRAMPOLINE
            }
        );
        assert_eq!(
            executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
            full_s1
        );
        assert_eq!(read_metadata(&mut executor), running);

        // Resume at the observed PC with a one-instruction budget. The only
        // way to arrive at the overflow publisher is by executing the JAL.
        let jump = executor
            .execute_until(check.exit_pc, &[normal, overflow], 1)
            .unwrap();
        assert_eq!(
            jump,
            ExecutionReport {
                executed_instructions: 1,
                exit_pc: overflow
            }
        );
        assert_eq!(read_metadata(&mut executor), running);
        println!("Overflow check: {check:?}; taken JAL: {jump:?}");

        let guest = finish_guest(
            &mut executor,
            ExecutionReport {
                executed_instructions: check.executed_instructions + jump.executed_instructions,
                exit_pc: jump.exit_pc,
            },
            GuestMetadata::new(
                GuestExecutionStatus::GuestException,
                PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE,
                full_s1,
                0,
                0,
                0,
            ),
            8,
        );
        assert_eq!(guest.stack_depth(), EVM_STACK_CAPACITY);
        assert_eq!(guest.stack(), initial_stack);
        let mut after_guard = [0; 32];
        executor.read_memory(full_s1, &mut after_guard).unwrap();
        assert_eq!(after_guard, guard);
    }
}

fn load_guest(artifact: &AssemblyResult) -> RvemuExecutor {
    let mut executor = RvemuExecutor::new().unwrap();
    executor.load_code(artifact.bytes()).unwrap();
    executor
        .write_memory(EXIT, &0x0010_0073_u32.to_le_bytes())
        .unwrap(); // EBREAK
    initialize_guest_state(&mut executor, 0).unwrap();
    executor
}

fn read_metadata(executor: &mut RvemuExecutor) -> GuestMetadata {
    let mut bytes = [0; GUEST_METADATA_ENCODED_SIZE];
    executor
        .read_memory(METADATA_REGION.start(), &mut bytes)
        .unwrap();
    GuestMetadata::decode(&bytes).unwrap()
}

fn finish_guest(
    executor: &mut RvemuExecutor,
    body: ExecutionReport,
    expected: GuestMetadata,
    publisher_count: u64,
) -> GuestResult {
    let publisher = body.exit_pc;
    let before_status = publisher + (publisher_count - 2) * 4;
    let mut pc = publisher;
    let mut count = 0;
    for (boundary, metadata) in [
        (
            publisher + 12,
            GuestMetadata::new(
                GuestExecutionStatus::Running,
                0,
                expected.final_s1(),
                0,
                0,
                0,
            ),
        ),
        (
            before_status,
            GuestMetadata::new(
                GuestExecutionStatus::Running,
                expected.exception_code(),
                expected.final_s1(),
                0,
                0,
                0,
            ),
        ),
        (before_status + 4, expected),
    ] {
        let limit = (boundary - pc) / 4;
        let step = executor.execute(pc, boundary, limit).unwrap();
        assert_eq!(step.executed_instructions, limit);
        assert_eq!(read_metadata(executor), metadata);
        println!(
            "Publication checkpoint {:#010x}: {metadata:?}",
            step.exit_pc
        );
        count += step.executed_instructions;
        pc = step.exit_pc;
    }
    let exit = executor.execute(pc, EXIT, 1).unwrap();
    assert_eq!(
        exit,
        ExecutionReport {
            executed_instructions: 1,
            exit_pc: EXIT
        }
    );
    count += exit.executed_instructions;
    assert_eq!(count, publisher_count);
    let guest = read_guest_result(executor, &exit, 0).unwrap();
    assert_eq!(guest.metadata().metadata(), expected);
    println!(
        "Reached publisher {publisher:#010x}, sentinel {EXIT:#010x}: {:?}, exception={}, depth={}, final_s1={:#010x}; RV64 body={} publisher={} total={}",
        guest.status(), guest.exception_code(), guest.stack_depth(), guest.final_s1(),
        body.executed_instructions, count, body.executed_instructions + count
    );
    guest
}

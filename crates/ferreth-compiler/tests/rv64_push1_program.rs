use alloy_primitives::U256;
use asm_rs::{assemble, Arch};
use ferreth_compiler::{
    compiler::{
        compile_evm_bytecode, CompileError, CompiledRv64Program,
        PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE,
    },
    execution::random_u256,
    orchestrator::{execute_guest_evm_bytecode, execute_guest_evm_bytecode_with_initial_stack},
    rv64::{
        guest::GuestExecutionStatus,
        memory::{EVM_STACK_CAPACITY, EVM_STACK_REGION, EVM_WORD_SIZE},
        opcodes::add::generate_guest_add,
        opcodes::push::generate_guest_push1,
        opcodes::sub::generate_guest_sub,
    },
};

#[test]
fn one_push1_produces_one_stack_word_and_publishes_s1() {
    let result = execute_guest_evm_bytecode(&[0x60, 5], u64::MAX).expect("execute one PUSH1");
    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(result.guest_result().stack(), &[U256::from(5)]);
    assert_eq!(
        result.guest_result().final_s1(),
        EVM_STACK_REGION.start() + 32
    );
}

#[test]
fn two_and_three_push1_operations_preserve_insertion_order_in_one_run() {
    let two = execute_guest_evm_bytecode(&[0x60, 5, 0x60, 3], u64::MAX)
        .expect("execute two PUSH1 operations");
    assert_eq!(two.guest_result().stack(), &[U256::from(5), U256::from(3)]);
    // 20 opcode instructions plus one shared seven-instruction normal exit,
    // rather than two separately executed 17-instruction programs.
    assert_eq!(two.instruction_metrics().total(), 29);

    let three = execute_guest_evm_bytecode(&[0x60, 1, 0x60, 2, 0x60, 3], u64::MAX)
        .expect("execute three PUSH1 operations");
    assert_eq!(
        three.guest_result().stack(),
        &[U256::from(1), U256::from(2), U256::from(3)]
    );
}

#[test]
fn zero_and_maximum_push1_immediates_are_guest_stack_words() {
    let result = execute_guest_evm_bytecode(&[0x60, 0, 0x60, u8::MAX], u64::MAX)
        .expect("execute byte boundaries");
    assert_eq!(
        result.guest_result().stack(),
        &[U256::ZERO, U256::from(u8::MAX)]
    );
}

#[test]
fn overflow_in_first_or_later_push1_is_exceptional_without_overflowing_write() {
    let full = (0..EVM_STACK_CAPACITY)
        .map(|value| U256::from(value + 1))
        .collect::<Vec<_>>();
    let first = execute_guest_evm_bytecode_with_initial_stack(&[0x60, 5], &full, u64::MAX)
        .expect("first PUSH1 overflow reaches guest exception exit");
    assert_eq!(
        first.guest_result().status(),
        GuestExecutionStatus::GuestException
    );
    assert_eq!(
        first.guest_result().exception_code(),
        PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE
    );
    assert_eq!(first.guest_result().stack(), full);

    let almost_full = full[..EVM_STACK_CAPACITY as usize - 1].to_vec();
    let later =
        execute_guest_evm_bytecode_with_initial_stack(&[0x60, 5, 0x60, 3], &almost_full, u64::MAX)
            .expect("later PUSH1 overflow reaches guest exception exit");
    let mut expected = almost_full;
    expected.push(U256::from(5));
    assert_eq!(
        later.guest_result().status(),
        GuestExecutionStatus::GuestException
    );
    assert_eq!(later.guest_result().stack(), expected);
    assert_eq!(later.guest_result().final_s1(), EVM_STACK_REGION.end());
}

#[test]
fn linked_program_has_no_fallthrough_to_handlers_or_unpatched_trampoline() {
    let bytecode = [0x60, 5, 0x60, 3];
    let program = compile_evm_bytecode(&bytecode).expect("compile PUSH1 program");
    assert!(!program.assembly().contains("jal zero, 0"));
    assert_eq!(
        assemble(program.assembly(), Arch::Rv64).expect("reassemble generated assembly"),
        program.machine_code()
    );
    let result = execute_guest_evm_bytecode(&bytecode, u64::MAX).expect("normal execution");
    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(result.guest_result().exception_code(), 0);
}

#[test]
fn bytecode_push1_programs_execute_in_one_rv64_run() {
    for (bytecode, expected) in [
        (&[0x60, 0x05][..], vec![U256::from(5)]),
        (
            &[0x60, 0x05, 0x60, 0x03][..],
            vec![U256::from(5), U256::from(3)],
        ),
        (
            &[0x60, 0x00, 0x60, 0xff][..],
            vec![U256::ZERO, U256::from(255)],
        ),
    ] {
        let result =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("compile and execute bytecode");
        assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(result.guest_result().stack(), expected);
    }

    let result = execute_guest_evm_bytecode(&[0x60, 5, 0x60, 3], u64::MAX).expect("two PUSH1s");
    assert_eq!(result.instruction_metrics().total(), 29);
}

#[test]
fn truncated_push1_zero_pads_missing_immediate() {
    let result =
        execute_guest_evm_bytecode(&[0x60], u64::MAX).expect("compile and execute truncated PUSH1");
    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(result.guest_result().exception_code(), 0);
    assert_eq!(result.guest_result().stack(), &[U256::ZERO]);
}

#[test]
fn explicit_stop_preserves_guest_state_and_uses_the_shared_normal_publisher() {
    let cases = [
        (&[0x00][..], vec![], EVM_STACK_REGION.start(), 10),
        (
            &[0x60, 0x05, 0x00][..],
            vec![U256::from(5)],
            EVM_STACK_REGION.start() + 32,
            20,
        ),
        (
            &[0x60, 0x05, 0x60, 0x03, 0x01, 0x00][..],
            vec![U256::from(8)],
            EVM_STACK_REGION.start() + 32,
            72,
        ),
        (
            &[0x60, 0x05, 0x60, 0x03, 0x03, 0x00][..],
            vec![U256::MAX - U256::from(1)],
            EVM_STACK_REGION.start() + 32,
            72,
        ),
    ];

    for (bytecode, expected_stack, expected_s1, expected_count) in cases {
        let first =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("STOP program executes once");
        let second =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("STOP program executes twice");
        assert_eq!(first.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(first.guest_result().exception_code(), 0);
        assert_eq!(first.guest_result().stack(), expected_stack);
        assert_eq!(first.guest_result().final_s1(), expected_s1);
        assert_eq!(first.instruction_metrics().guest_termination(), 9);
        assert_eq!(first.instruction_metrics().total(), expected_count);
        assert_eq!(first.instruction_metrics(), second.instruction_metrics());
    }

    let stop = compile_evm_bytecode(&[0x00]).expect("compile STOP");
    assert_eq!(stop.assembly().matches("normal_exit:").count(), 1);
    assert!(stop
        .assembly()
        .contains("guest_stop:\n    jal zero, normal_exit\nnormal_exit:"));
    assert_eq!(
        u32::from_le_bytes(stop.machine_code()[0..4].try_into().unwrap()),
        0x0040_006f
    );
    assert_eq!(
        assemble(stop.assembly(), Arch::Rv64).expect("reassemble STOP"),
        stop.machine_code()
    );
}

#[test]
fn explicit_stop_does_not_execute_trailing_bytecode() {
    let result = execute_guest_evm_bytecode(&[0x60, 0x05, 0x00, 0x60, 0x09], u64::MAX)
        .expect("STOP bypasses the trailing PUSH1");
    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(result.guest_result().stack(), &[U256::from(5)]);
    assert_eq!(
        result.guest_result().final_s1(),
        EVM_STACK_REGION.start() + 32
    );
    // Successful completion also establishes that the EBREAK installed at the
    // exit sentinel was not fetched or executed.
    assert_eq!(result.instruction_metrics().total(), 20);
}

#[test]
fn bytecode_push1_and_add_programs_execute_as_one_linked_rv64_program() {
    for (bytecode, expected) in [
        (&[0x60, 5, 0x60, 3, 0x01][..], vec![U256::from(8)]),
        (&[0x60, 1, 0x60, 2, 0x01][..], vec![U256::from(3)]),
        (
            &[0x60, 5, 0x60, 3, 0x01, 0x60, 2, 0x01][..],
            vec![U256::from(10)],
        ),
    ] {
        let first =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("compile and execute bytecode");
        let second =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("repeat bytecode execution");
        assert_eq!(first.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(first.guest_result().exception_code(), 0);
        assert_eq!(first.guest_result().stack(), expected);
        assert_eq!(first.instruction_metrics(), second.instruction_metrics());
    }

    let result = execute_guest_evm_bytecode(&[0x60, 5, 0x60, 3, 0x01], u64::MAX)
        .expect("PUSH1 PUSH1 ADD executes once");
    assert_eq!(result.instruction_metrics().opcode_body(), 62);
    assert_eq!(result.instruction_metrics().guest_termination(), 9);
    assert_eq!(result.instruction_metrics().total(), 71);
}

#[test]
fn compiled_add_underflow_publishes_terminal_metadata_without_mutating_stack() {
    for initial_stack in [vec![], vec![U256::from(9)]] {
        let expected_s1 = EVM_STACK_REGION.start() + initial_stack.len() as u64 * 32;
        let result =
            execute_guest_evm_bytecode_with_initial_stack(&[0x01], &initial_stack, u64::MAX)
                .expect("ADD underflow reaches the shared guest publisher");
        assert_eq!(
            result.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(
            result.guest_result().exception_code(),
            ferreth_compiler::compiler::PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE
        );
        assert_eq!(result.guest_result().final_s1(), expected_s1);
        assert_eq!(result.guest_result().stack(), initial_stack);
        assert_eq!(result.instruction_metrics().total(), 13);
    }
}

#[test]
fn bytecode_push1_and_sub_programs_execute_as_one_linked_rv64_program() {
    let maximum = U256::MAX;
    for (bytecode, expected) in [
        (&[0x60, 5, 0x60, 3, 0x03][..], vec![maximum - U256::from(1)]),
        (&[0x60, 3, 0x60, 5, 0x03][..], vec![U256::from(2)]),
        (&[0x60, 5, 0x60, 5, 0x03][..], vec![U256::ZERO]),
        (
            &[0x60, 5, 0x60, 3, 0x01, 0x60, 2, 0x03][..],
            vec![maximum - U256::from(5)],
        ),
    ] {
        let first =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("compile and execute bytecode");
        let second =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("repeat bytecode execution");
        assert_eq!(first.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(first.guest_result().exception_code(), 0);
        assert_eq!(first.guest_result().stack(), expected);
        assert_eq!(first.instruction_metrics(), second.instruction_metrics());
    }
}

#[test]
fn compiled_sub_underflow_publishes_terminal_metadata_without_mutating_stack() {
    for initial_stack in [vec![], vec![U256::from(9)]] {
        let expected_s1 = EVM_STACK_REGION.start() + initial_stack.len() as u64 * 32;
        let result =
            execute_guest_evm_bytecode_with_initial_stack(&[0x03], &initial_stack, u64::MAX)
                .expect("SUB underflow reaches the shared guest publisher");
        assert_eq!(
            result.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(
            result.guest_result().exception_code(),
            ferreth_compiler::compiler::PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE
        );
        assert_eq!(result.guest_result().final_s1(), expected_s1);
        assert_eq!(result.guest_result().stack(), initial_stack);
        assert_eq!(result.instruction_metrics().total(), 13);
    }
}

#[test]
fn mixed_linker_keeps_the_generated_add_core_machine_code_unchanged() {
    let generated = generate_guest_add();
    let compiled = compile_evm_bytecode(&[0x60, 5, 0x60, 3, 0x01]).expect("compile mixed program");
    let add_start = 2 * 11 * 4;
    // The continuation is the end of the generated body; the core starts at
    // the documented fixed wrapper width (nine instructions).
    let core_start = add_start + 9 * 4;
    assert_eq!(
        &compiled.machine_code()[core_start..core_start + generated.machine_code().len() - 9 * 4],
        &generated.machine_code()[9 * 4..]
    );
}

#[test]
fn mixed_linker_keeps_the_generated_sub_core_machine_code_unchanged() {
    let generated = generate_guest_sub();
    let compiled = compile_evm_bytecode(&[0x60, 5, 0x60, 3, 0x03]).expect("compile mixed program");
    let sub_start = 2 * 11 * 4;
    let core_start = sub_start + 9 * 4;
    assert_eq!(
        &compiled.machine_code()[core_start..core_start + generated.machine_code().len() - 9 * 4],
        &generated.machine_code()[9 * 4..]
    );
    assert!(!compiled.assembly().contains("jal zero, 0"));
    assert_eq!(
        assemble(compiled.assembly(), Arch::Rv64).expect("reassemble generated assembly"),
        compiled.machine_code()
    );
}

#[test]
fn push1_execution_uses_the_compiled_bytecode_image() {
    let bytecode = [0x60, 5, 0x60, 3];
    let compiled = compile_evm_bytecode(&bytecode).expect("compile bytecode");
    let executed = execute_guest_evm_bytecode(&bytecode, u64::MAX).expect("execute bytecode");
    assert_eq!(compiled.machine_code(), executed.machine_code());
    assert_eq!(compiled.assembly(), executed.assembly());
}

#[test]
fn bytecode_compilation_rejects_complete_and_truncated_push2() {
    for bytecode in [&[0x60, 0, 0x61, 0, 1][..], &[0x60, 0, 0x61, 0x05][..]] {
        assert_eq!(
            compile_evm_bytecode(bytecode),
            Err(CompileError::UnsupportedInstruction {
                opcode: 0x61,
                offset: 2,
            })
        );
    }
}

fn repeated_push1_bytecode(pushes: usize, explicit_stop: bool) -> Vec<u8> {
    let mut bytecode = [0x60, 0x02].repeat(pushes);
    if explicit_stop {
        bytecode.push(0x00);
    }
    bytecode
}

fn assert_first_push_reaches_overflow_publisher(program: &CompiledRv64Program) {
    let trampoline = generate_guest_push1(2).overflow_trampoline_offset();
    let word = u32::from_le_bytes(
        program.machine_code()[trampoline..trampoline + 4]
            .try_into()
            .expect("first PUSH1 trampoline is a complete instruction"),
    );
    assert_eq!(word & 0x7f, 0x6f, "overflow trampoline must be JAL");
    assert_eq!((word >> 7) & 0x1f, 0, "overflow JAL must use x0");

    // J-type immediate bits are scattered across the instruction word.
    let immediate = ((word >> 31) << 20)
        | (((word >> 12) & 0xff) << 12)
        | (((word >> 20) & 1) << 11)
        | (((word >> 21) & 0x3ff) << 1);
    let displacement = i64::from((immediate as i32) << 11 >> 11);
    let trampoline_pc = program.entry_pc() + trampoline as u64;
    let target_pc = trampoline_pc
        .checked_add_signed(displacement)
        .expect("overflow destination fits in a PC");
    // Both bytecode paths list the normal publisher first and overflow second.
    assert_eq!(target_pc, program.termination_entry_pcs()[1]);
}

fn assert_repeated_push1_succeeds(pushes: usize, explicit_stop: bool) {
    let bytecode = repeated_push1_bytecode(pushes, explicit_stop);
    let program = compile_evm_bytecode(&bytecode).expect("compile repeated PUSH1 bytecode");
    assert_first_push_reaches_overflow_publisher(&program);

    let execution =
        execute_guest_evm_bytecode(&bytecode, u64::MAX).expect("execute repeated PUSH1 bytecode");
    let result = execution.guest_result();
    assert_eq!(result.status(), GuestExecutionStatus::Normal);
    assert_eq!(result.exception_code(), 0);
    assert_eq!(result.stack(), vec![U256::from(2); pushes]);
    assert_eq!(result.stack_depth(), pushes as u64);
    assert_eq!(
        result.final_s1(),
        EVM_STACK_REGION.start() + pushes as u64 * EVM_WORD_SIZE
    );
}

fn assert_repeated_push1_overflows(explicit_stop: bool) {
    let bytecode = repeated_push1_bytecode(47, explicit_stop);
    let program = compile_evm_bytecode(&bytecode).expect("compile repeated PUSH1 bytecode");
    assert_first_push_reaches_overflow_publisher(&program);

    let mut seed = 42;
    let initial_stack = (0..EVM_STACK_CAPACITY)
        .map(|_| random_u256(&mut seed))
        .collect::<Vec<_>>();
    let execution =
        execute_guest_evm_bytecode_with_initial_stack(&bytecode, &initial_stack, u64::MAX)
            .expect("execute repeated PUSH1 bytecode on a full stack");
    let result = execution.guest_result();
    assert_eq!(result.status(), GuestExecutionStatus::GuestException);
    assert_eq!(
        result.exception_code(),
        PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE
    );
    assert_eq!(result.stack(), initial_stack);
    assert_eq!(result.stack_depth(), EVM_STACK_CAPACITY);
    assert_eq!(result.final_s1(), EVM_STACK_REGION.end());
}

#[test]
fn repeated_push1_46_eof_completes_normally() {
    assert_repeated_push1_succeeds(46, false);
}

#[test]
fn repeated_push1_46_stop_completes_normally() {
    assert_repeated_push1_succeeds(46, true);
}

// LF-026 regresses at 47 PUSH1s for both compiler paths.
#[test]
fn repeated_push1_47_eof_completes_normally() {
    assert_repeated_push1_succeeds(47, false);
}

#[test]
fn repeated_push1_47_stop_completes_normally() {
    assert_repeated_push1_succeeds(47, true);
}

#[test]
fn repeated_push1_47_eof_full_stack_publishes_overflow() {
    assert_repeated_push1_overflows(false);
}

#[test]
fn repeated_push1_47_stop_full_stack_publishes_overflow() {
    assert_repeated_push1_overflows(true);
}

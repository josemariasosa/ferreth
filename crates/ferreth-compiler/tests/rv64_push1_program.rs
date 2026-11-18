use asm_rs::{assemble, Arch};
use ferreth_compiler::{
    compiler::{
        compile_evm_bytecode, compile_push1_program, CompileError,
        PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE,
    },
    orchestrator::{
        execute_guest_evm_bytecode, execute_guest_evm_bytecode_with_initial_stack,
        execute_guest_push1_sequence, execute_guest_push1_sequence_with_initial_stack,
    },
    rv64::{
        guest::GuestExecutionStatus,
        memory::{EVM_STACK_CAPACITY, EVM_STACK_REGION},
        opcodes::add::generate_guest_add,
        opcodes::sub::generate_guest_sub,
    },
};
use revm::primitives::U256;

#[test]
fn one_push1_produces_one_stack_word_and_publishes_s1() {
    let result = execute_guest_push1_sequence(&[5]).expect("execute one PUSH1");
    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(result.guest_result().stack(), &[U256::from(5)]);
    assert_eq!(
        result.guest_result().final_s1(),
        EVM_STACK_REGION.start() + 32
    );
}

#[test]
fn two_and_three_push1_operations_preserve_insertion_order_in_one_run() {
    let two = execute_guest_push1_sequence(&[5, 3]).expect("execute two PUSH1 operations");
    assert_eq!(two.guest_result().stack(), &[U256::from(5), U256::from(3)]);
    // 20 opcode instructions plus one shared seven-instruction normal exit,
    // rather than two separately executed 17-instruction programs.
    assert_eq!(two.instruction_metrics().total(), 27);

    let three = execute_guest_push1_sequence(&[1, 2, 3]).expect("execute three PUSH1 operations");
    assert_eq!(
        three.guest_result().stack(),
        &[U256::from(1), U256::from(2), U256::from(3)]
    );
}

#[test]
fn zero_and_maximum_push1_immediates_are_guest_stack_words() {
    let result = execute_guest_push1_sequence(&[0, u8::MAX]).expect("execute byte boundaries");
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
    let first = execute_guest_push1_sequence_with_initial_stack(&[5], &full)
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
    let later = execute_guest_push1_sequence_with_initial_stack(&[5, 3], &almost_full)
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
    let program = compile_push1_program(&[5, 3]).expect("compile PUSH1 program");
    assert!(!program.assembly().contains("jal zero, 0"));
    assert_eq!(
        assemble(program.assembly(), Arch::Rv64).expect("reassemble generated assembly"),
        program.machine_code()
    );
    let result = execute_guest_push1_sequence(&[5, 3]).expect("normal execution");
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
        let result = execute_guest_evm_bytecode(bytecode).expect("compile and execute bytecode");
        assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(result.guest_result().stack(), expected);
    }

    let result = execute_guest_evm_bytecode(&[0x60, 5, 0x60, 3]).expect("two PUSH1s");
    assert_eq!(result.instruction_metrics().total(), 27);
}

#[test]
fn explicit_stop_preserves_guest_state_and_uses_the_shared_normal_publisher() {
    let cases = [
        (&[0x00][..], vec![], EVM_STACK_REGION.start(), 8),
        (
            &[0x60, 0x05, 0x00][..],
            vec![U256::from(5)],
            EVM_STACK_REGION.start() + 32,
            18,
        ),
        (
            &[0x60, 0x05, 0x60, 0x03, 0x01, 0x00][..],
            vec![U256::from(8)],
            EVM_STACK_REGION.start() + 32,
            70,
        ),
        (
            &[0x60, 0x05, 0x60, 0x03, 0x03, 0x00][..],
            vec![U256::MAX - U256::from(1)],
            EVM_STACK_REGION.start() + 32,
            70,
        ),
    ];

    for (bytecode, expected_stack, expected_s1, expected_count) in cases {
        let first = execute_guest_evm_bytecode(bytecode).expect("STOP program executes once");
        let second = execute_guest_evm_bytecode(bytecode).expect("STOP program executes twice");
        assert_eq!(first.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(first.guest_result().exception_code(), 0);
        assert_eq!(first.guest_result().stack(), expected_stack);
        assert_eq!(first.guest_result().final_s1(), expected_s1);
        assert_eq!(first.instruction_metrics().guest_termination(), 7);
        assert_eq!(first.instruction_metrics().total(), expected_count);
        assert_eq!(first.instruction_metrics(), second.instruction_metrics());
    }

    let stop = compile_evm_bytecode(&[0x00]).expect("compile STOP");
    assert_eq!(stop.assembly().matches("normal_exit:").count(), 1);
    assert!(stop
        .assembly()
        .contains("evm_stop:\n    jal zero, 4\nnormal_exit:"));
}

#[test]
fn explicit_stop_does_not_execute_trailing_bytecode() {
    let result = execute_guest_evm_bytecode(&[0x60, 0x05, 0x00, 0x60, 0x09])
        .expect("STOP bypasses the trailing PUSH1");
    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(result.guest_result().stack(), &[U256::from(5)]);
    assert_eq!(
        result.guest_result().final_s1(),
        EVM_STACK_REGION.start() + 32
    );
    // Successful completion also establishes that the EBREAK installed at the
    // exit sentinel was not fetched or executed.
    assert_eq!(result.instruction_metrics().total(), 18);
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
        let first = execute_guest_evm_bytecode(bytecode).expect("compile and execute bytecode");
        let second = execute_guest_evm_bytecode(bytecode).expect("repeat bytecode execution");
        assert_eq!(first.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(first.guest_result().exception_code(), 0);
        assert_eq!(first.guest_result().stack(), expected);
        assert_eq!(first.instruction_metrics(), second.instruction_metrics());
    }

    let result = execute_guest_evm_bytecode(&[0x60, 5, 0x60, 3, 0x01])
        .expect("PUSH1 PUSH1 ADD executes once");
    assert_eq!(result.instruction_metrics().opcode_body(), 62);
    assert_eq!(result.instruction_metrics().guest_termination(), 7);
    assert_eq!(result.instruction_metrics().total(), 69);
}

#[test]
fn compiled_add_underflow_publishes_terminal_metadata_without_mutating_stack() {
    for initial_stack in [vec![], vec![U256::from(9)]] {
        let expected_s1 = EVM_STACK_REGION.start() + initial_stack.len() as u64 * 32;
        let result = execute_guest_evm_bytecode_with_initial_stack(&[0x01], &initial_stack)
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
        assert_eq!(result.instruction_metrics().total(), 11);
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
        let first = execute_guest_evm_bytecode(bytecode).expect("compile and execute bytecode");
        let second = execute_guest_evm_bytecode(bytecode).expect("repeat bytecode execution");
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
        let result = execute_guest_evm_bytecode_with_initial_stack(&[0x03], &initial_stack)
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
        assert_eq!(result.instruction_metrics().total(), 11);
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
fn bytecode_compilation_reuses_the_push1_program_linker() {
    let from_bytecode = compile_evm_bytecode(&[0x60, 5, 0x60, 3]).expect("compile bytecode");
    let from_sequence = compile_push1_program(&[5, 3]).expect("compile sequence");
    assert_eq!(from_bytecode.machine_code(), from_sequence.machine_code());
    assert_eq!(from_bytecode.assembly(), from_sequence.assembly());
}

#[test]
fn bytecode_rejects_unimplemented_or_malformed_input_before_execution() {
    let (bytecode, opcode, offset) = (&[0x61, 0, 1][..], 0x61, 0);
    assert_eq!(
        compile_evm_bytecode(bytecode),
        Err(CompileError::UnsupportedInstruction { opcode, offset })
    );

    assert!(matches!(
        compile_evm_bytecode(&[0x61, 0x05]),
        Err(CompileError::Decode(_))
    ));
}

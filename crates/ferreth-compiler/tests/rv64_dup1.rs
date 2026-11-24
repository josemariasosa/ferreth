use alloy_primitives::U256;
use asm_rs::{assemble, Arch};
use ferreth_compiler::{
    compiler::{compile_evm_bytecode, PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE},
    decode,
    execution::ProgramRunner,
    orchestrator::{
        execute_guest_dup1, execute_guest_dup1_with_initial_stack, execute_guest_evm_bytecode,
    },
    rv64::{
        executor::{IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{initialize_guest_state, read_guest_result, write_evm_word, GuestExecutionStatus},
        memory::{ENTRY_PC, EVM_STACK_CAPACITY, EVM_STACK_REGION, EXIT_PC},
    },
};

#[test]
fn dup1_executes_in_supported_bytecode_sequences() {
    let cases = [
        (&[0x60, 0x05, 0x80][..], vec![U256::from(5), U256::from(5)]),
        (&[0x60, 0x05, 0x80, 0x01], vec![U256::from(10)]),
        (
            &[0x60, 0x05, 0x60, 0x03, 0x80],
            vec![U256::from(5), U256::from(3), U256::from(3)],
        ),
        (&[0x60, 0x05, 0x80, 0x50], vec![U256::from(5)]),
        (&[0x60, 0x05, 0x80, 0x03, 0x00], vec![U256::ZERO]),
        (&[0x60, 0x05, 0x00, 0x80], vec![U256::from(5)]),
    ];

    for (bytecode, expected_stack) in cases {
        let result =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("execute linked DUP1 bytecode");
        assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(result.guest_result().exception_code(), 0);
        assert_eq!(result.guest_result().stack(), expected_stack);
        assert_eq!(
            result.guest_result().final_s1(),
            EVM_STACK_REGION.start() + expected_stack.len() as u64 * 32
        );
    }
}

#[test]
fn dup1_underflow_publishes_exception_without_mutating_the_stack() {
    let result =
        execute_guest_evm_bytecode(&[0x80], u64::MAX).expect("DUP1 underflow is guest-published");
    assert_eq!(
        result.guest_result().status(),
        GuestExecutionStatus::GuestException
    );
    assert_eq!(
        result.guest_result().exception_code(),
        ferreth_compiler::compiler::PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE
    );
    assert_eq!(result.guest_result().stack(), &[] as &[U256]);
    assert_eq!(result.guest_result().final_s1(), EVM_STACK_REGION.start());
}

#[test]
fn dup1_copies_all_four_limbs_and_preserves_the_original() {
    let word = U256::from_limbs([
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210,
        0x0f1e_2d3c_4b5a_6978,
        0x8877_6655_4433_2211,
    ]);
    let lower = U256::from(17);
    let result = execute_guest_dup1_with_initial_stack(&[lower, word])
        .expect("execute isolated DUP1 with a multi-limb word");

    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(
        result.guest_result().stack(),
        &[lower, word, word],
        "all four limbs must be copied without modifying the source"
    );
    assert_eq!(
        result.guest_result().final_s1(),
        EVM_STACK_REGION.start() + 3 * 32
    );
}

#[test]
fn dup1_overflow_does_not_write_the_next_slot_or_advance_s1() {
    let compiled = compile_evm_bytecode(&[0x80]).expect("compile DUP1");
    let mut executor = RvemuExecutor::new().expect("initialize rvemu");
    executor
        .load_code(compiled.machine_code())
        .expect("load generated code");
    initialize_guest_state(&mut executor, 3).expect("initialize guest state");

    let full_stack = (0..EVM_STACK_CAPACITY)
        .map(|index| U256::from(index + 1))
        .collect::<Vec<_>>();
    for (index, word) in full_stack.iter().copied().enumerate() {
        write_evm_word(&mut executor, index, word).expect("write initial stack word");
    }
    let next_slot_sentinel = [0xa5; 32];
    executor
        .write_memory(EVM_STACK_REGION.end(), &next_slot_sentinel)
        .expect("write sentinel after the stack region");
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, EVM_STACK_REGION.end());

    let report = executor
        .execute(ENTRY_PC, EXIT_PC, compiled.instruction_limit())
        .expect("DUP1 overflow reaches the guest publisher");
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        EVM_STACK_REGION.start()
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        EVM_STACK_REGION.end()
    );
    let mut unchanged_sentinel = [0; 32];
    executor
        .read_memory(EVM_STACK_REGION.end(), &mut unchanged_sentinel)
        .expect("read sentinel after the stack region");
    assert_eq!(unchanged_sentinel, next_slot_sentinel);

    let result = read_guest_result(&mut executor, &report, 3).expect("validate overflow metadata");
    assert_eq!(result.status(), GuestExecutionStatus::GuestException);
    assert_eq!(
        result.exception_code(),
        PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE
    );
    assert_eq!(result.final_s1(), EVM_STACK_REGION.end());
    assert_eq!(result.stack(), full_stack);
    assert_eq!(result.remaining_gas(), 0);
    assert_eq!(result.gas_used(), 3);
}

#[test]
fn rc_ro_and_generator_share_the_linked_rv64_behavior() {
    let isolated = execute_guest_dup1().expect("execute isolated DUP1");
    let compiled =
        execute_guest_evm_bytecode(&[0x60, 0x00, 0x80], u64::MAX).expect("execute RC DUP1");
    assert_eq!(
        isolated.guest_result().status(),
        GuestExecutionStatus::Normal
    );
    assert_eq!(isolated.guest_result().stack(), &[U256::ZERO, U256::ZERO]);
    assert_eq!(compiled.guest_result().stack(), &[U256::ZERO, U256::ZERO]);

    assert_eq!(isolated.guest_result().remaining_gas(), u64::MAX - 3);
    assert_eq!(isolated.guest_result().gas_used(), 3);
    assert_eq!(compiled.guest_result().gas_used(), 6);

    let generated = ferreth_compiler::rv64::opcodes::dup::generate_guest_dup1();
    assert_eq!(
        assemble(generated.assembly(), Arch::Rv64).expect("reassemble DUP1"),
        generated.machine_code()
    );
}

#[test]
fn legacy_host_stack_runner_rejects_dup1() {
    let mut runner = ProgramRunner::new().expect("initialize legacy arithmetic runner");
    let instructions = decode(&[0x60, 0x05, 0x80]).expect("decode DUP1 bytecode");
    assert_eq!(
        runner.run(&instructions).unwrap_err(),
        "DUP1 requires the stack-aware RV64 guest execution path"
    );
}

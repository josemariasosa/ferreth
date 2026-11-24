use alloy_primitives::U256;
use ferreth_compiler::{
    compiler::PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
    orchestrator::{
        execute_guest_evm_bytecode, execute_guest_evm_bytecode_with_initial_stack,
        execute_guest_pop_with_initial_stack,
    },
    rv64::{
        guest::GuestExecutionStatus, memory::EVM_STACK_REGION, opcodes::pop::generate_guest_pop,
    },
};

#[test]
fn pop_generator_is_a_minimal_stack_pointer_only_body() {
    let generated = generate_guest_pop();

    assert_eq!(generated.machine_code().len(), 16);
    assert!(generated.assembly().contains("bgeu s0, s1"));
    assert!(generated.assembly().contains("addi s1, s1, -32"));
    assert!(!generated.assembly().contains("addi s0,"));
    assert!(!generated.assembly().contains("add s0,"));
    assert!(!generated.assembly().contains("ld "));
    assert!(!generated.assembly().contains("sd "));
}

#[test]
fn pop_programs_update_only_the_live_stack_extent() {
    let cases = [
        (&[0x60, 0x05, 0x60, 0x03, 0x50][..], vec![U256::from(5)]),
        (&[0x60, 0x50, 0x50][..], vec![]),
        (&[0x60, 0x05, 0x50, 0x00][..], vec![]),
    ];
    for (bytecode, expected_stack) in cases {
        let result = execute_guest_evm_bytecode(bytecode, u64::MAX).expect("POP program executes");
        assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(result.guest_result().stack(), expected_stack);
        assert_eq!(
            result.guest_result().final_s1(),
            EVM_STACK_REGION.start() + u64::try_from(expected_stack.len()).unwrap() * 32
        );
    }

    let initial_stack = [U256::from(5), U256::from(3), U256::from(9)];
    let result =
        execute_guest_pop_with_initial_stack(&initial_stack).expect("isolated POP executes");
    assert_eq!(result.guest_result().stack(), &initial_stack[..2]);
    assert_eq!(
        result.guest_result().final_s1(),
        EVM_STACK_REGION.start() + 64
    );
}

#[test]
fn empty_pop_publishes_underflow_without_changing_s1_or_live_words() {
    for (bytecode, expected_final_s1) in [
        (&[0x50][..], EVM_STACK_REGION.start()),
        (&[0x60, 0x05, 0x50, 0x50][..], EVM_STACK_REGION.start()),
        (
            &[0x60, 0x05, 0x60, 0x03, 0x50, 0x01][..],
            EVM_STACK_REGION.start() + 32,
        ),
    ] {
        let result =
            execute_guest_evm_bytecode(bytecode, u64::MAX).expect("underflow is a guest result");
        assert_eq!(
            result.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(
            result.guest_result().exception_code(),
            PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE
        );
        assert_eq!(result.guest_result().final_s1(), expected_final_s1);
    }

    let initial_stack = [U256::from(5)];
    let result =
        execute_guest_evm_bytecode_with_initial_stack(&[0x50, 0x50], &initial_stack, u64::MAX)
            .expect("second POP underflows");
    assert_eq!(result.guest_result().stack(), &[] as &[U256]);
    assert_eq!(result.guest_result().final_s1(), EVM_STACK_REGION.start());
}

#[test]
fn pop_after_stop_is_unreachable() {
    let result = execute_guest_evm_bytecode(&[0x60, 0x05, 0x00, 0x50], u64::MAX)
        .expect("STOP makes trailing POP unreachable");
    assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
    assert_eq!(result.guest_result().stack(), &[U256::from(5)]);
    assert_eq!(
        result.guest_result().final_s1(),
        EVM_STACK_REGION.start() + 32
    );
}

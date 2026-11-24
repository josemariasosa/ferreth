use alloy_primitives::U256;
use asm_rs::{assemble, Arch};
use ferreth_compiler::{
    orchestrator::{
        execute_guest_add, execute_guest_add_with_initial_stack,
        PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
    },
    rv64::{guest::GuestExecutionStatus, memory::EVM_STACK_REGION},
};

#[test]
fn guest_add_publishes_the_rv64_computed_result() {
    for (a, b, expected) in [
        (U256::from(5), U256::from(3), U256::from(8)),
        (U256::ZERO, U256::ZERO, U256::ZERO),
        (
            U256::from_limbs([u64::MAX, u64::MAX, 0, 0]),
            U256::from(1),
            U256::from_limbs([0, 0, 1, 0]),
        ),
        (U256::MAX, U256::from(1), U256::ZERO),
    ] {
        let result = execute_guest_add(a, b).expect("ADD completes through the RV64 guest");

        assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(result.guest_result().exception_code(), 0);
        assert_eq!(result.guest_result().stack(), &[expected]);
        assert_eq!(result.guest_result().stack_depth(), 1);
        assert_eq!(
            result.guest_result().final_s1(),
            EVM_STACK_REGION.start() + 32
        );
    }
}

#[test]
fn guest_add_underflow_publishes_an_unchanged_stack_and_s1() {
    for initial_stack in [vec![], vec![U256::from(9)]] {
        let expected_s1 = EVM_STACK_REGION.start() + initial_stack.len() as u64 * 32;
        let result = execute_guest_add_with_initial_stack(&initial_stack)
            .expect("underflow reaches the guest exception exit");

        assert_eq!(
            result.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(
            result.guest_result().exception_code(),
            PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE
        );
        assert_eq!(result.guest_result().stack(), initial_stack);
        assert_eq!(result.guest_result().final_s1(), expected_s1);
    }
}

#[test]
fn guest_add_links_both_exit_paths_and_has_deterministic_instruction_counts() {
    let first = execute_guest_add(U256::from(5), U256::from(3)).expect("first ADD execution");
    let second = execute_guest_add(U256::from(5), U256::from(3)).expect("second ADD execution");
    assert_eq!(first.instruction_metrics(), second.instruction_metrics());
    assert_eq!(first.instruction_metrics().total(), 51);
    assert_eq!(first.instruction_metrics().guest_termination(), 9);
    assert_eq!(first.instruction_metrics().opcode_body(), 42);
    assert!(!first.assembly().contains("jal zero, 0"));
    assert_eq!(
        assemble(first.assembly(), Arch::Rv64).expect("reassemble linked ADD"),
        first.machine_code()
    );

    let underflow = execute_guest_add_with_initial_stack(&[]).expect("underflow ADD execution");
    assert_eq!(underflow.instruction_metrics().total(), 13);
    assert_eq!(underflow.instruction_metrics().guest_termination(), 10);
    assert_eq!(underflow.instruction_metrics().opcode_body(), 3);
}

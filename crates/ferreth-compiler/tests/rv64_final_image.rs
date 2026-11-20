use alloy_primitives::U256;
use ferreth_compiler::{
    compiler::{compile_evm_bytecode, compile_push1_program},
    orchestrator::{
        execute_guest_add_with_initial_stack, execute_guest_evm_bytecode,
        execute_guest_push1_sequence, execute_guest_sub_with_initial_stack,
    },
    rv64::{guest::GuestExecutionStatus, memory::ENTRY_PC},
};

#[test]
fn verified_push_only_and_mixed_images_keep_public_artifacts_and_execution() {
    let push = compile_push1_program(&[2]).unwrap();
    assert_eq!(push, compile_evm_bytecode(&[0x60, 2]).unwrap());
    assert_eq!(push.termination_entry_pcs(), [ENTRY_PC + 44, ENTRY_PC + 72]);
    let executed = execute_guest_push1_sequence(&[2]).unwrap();
    assert_eq!(executed.machine_code(), push.machine_code());
    assert_eq!(executed.assembly(), push.assembly());
    assert_eq!(executed.instruction_metrics().total(), 17);

    let bytecode = [0x60, 2, 0x80, 0x50, 0x60, 3, 0x01, 0x60, 7, 0x03, 0x00];
    let mixed = compile_evm_bytecode(&bytecode).unwrap();
    assert_eq!(
        mixed.termination_entry_pcs(),
        [ENTRY_PC + 560, ENTRY_PC + 588, ENTRY_PC + 620]
    );
    assert_eq!(mixed.machine_code().len(), 652);
    let executed = execute_guest_evm_bytecode(&bytecode).unwrap();
    assert_eq!(executed.machine_code(), mixed.machine_code());
    assert_eq!(executed.assembly(), mixed.assembly());
    assert_eq!(
        executed.guest_result().status(),
        GuestExecutionStatus::Normal
    );
    assert_eq!(executed.guest_result().stack(), &[U256::from(2)]);
    assert_eq!(executed.instruction_metrics().opcode_body(), 132);
    assert_eq!(executed.instruction_metrics().guest_termination(), 7);
    assert_eq!(executed.instruction_metrics().total(), 139);
}

#[test]
fn verified_isolated_arithmetic_keeps_two_publishers_for_both_paths() {
    let operands = [U256::from(2), U256::from(3)];
    for (execute, expected) in [
        (execute_guest_add_with_initial_stack as fn(&[U256]) -> _, 5),
        (execute_guest_sub_with_initial_stack, 1),
    ] {
        let normal = execute(&operands).unwrap();
        let underflow = execute(&[]).unwrap();
        assert_eq!(normal.machine_code(), underflow.machine_code());
        assert_eq!(normal.assembly(), underflow.assembly());
        assert_eq!(normal.machine_code().len(), 232);
        assert_eq!(normal.assembly().matches("jalr zero, -4(t1)").count(), 2);
        assert_eq!(normal.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(normal.guest_result().stack(), &[U256::from(expected)]);
        assert_eq!(normal.instruction_metrics().total(), 49);
        assert_eq!(
            underflow.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(underflow.guest_result().exception_code(), 2);
        assert_eq!(underflow.instruction_metrics().total(), 11);
    }
}

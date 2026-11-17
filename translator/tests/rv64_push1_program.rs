use asm_rs::{assemble, Arch};
use ferrum_eth_translator::{
    compiler::{compile_push1_program, PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE},
    orchestrator::{execute_guest_push1_sequence, execute_guest_push1_sequence_with_initial_stack},
    rv64::{
        guest::GuestExecutionStatus,
        memory::{EVM_STACK_CAPACITY, EVM_STACK_REGION},
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

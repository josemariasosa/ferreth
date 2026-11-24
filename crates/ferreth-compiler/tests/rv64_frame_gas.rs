use std::process::Command;

use ferreth_compiler::{
    orchestrator::execute_guest_evm_bytecode,
    rv64::{guest::GuestExecutionStatus, memory::EVM_STACK_REGION},
};

#[test]
fn stop_and_eof_publish_the_entire_budget() {
    for code in [&[0][..], &[]] {
        for limit in [0, 1, 7, 1 << 63, u64::MAX] {
            let execution = execute_guest_evm_bytecode(code, limit).unwrap();
            let guest = execution.guest_result();
            assert_eq!(guest.status(), GuestExecutionStatus::Normal);
            assert_eq!(guest.exception_code(), 0);
            assert!(guest.stack().is_empty());
            assert_eq!(guest.final_s1(), EVM_STACK_REGION.start());
            assert_eq!(guest.gas_limit(), limit);
            assert_eq!(guest.remaining_gas(), limit);
            assert_eq!(guest.gas_used(), 0);
        }
    }
}

#[test]
fn cli_passes_resolved_budget_to_guest() {
    for (args, expected) in [
        (&[][..], u64::MAX),
        (&["--gas", "0"], 0),
        (&["--gas", "7"], 7),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ferreth-compiler"))
            .args(["execute", "--bytecode", "00"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout
                .lines()
                .any(|line| line == format!("Remaining frame gas: {expected}")),
            "{stdout}"
        );
    }
}

fn assert_push_result(
    execution: &ferreth_compiler::orchestrator::OrchestratorResult,
    limit: u64,
    code: u64,
    remaining: u64,
    values: &[u64],
) {
    use alloy_primitives::U256;
    use ferreth_compiler::rv64::memory::EVM_WORD_SIZE;

    let guest = execution.guest_result();
    let status = if code == 0 {
        GuestExecutionStatus::Normal
    } else {
        GuestExecutionStatus::GuestException
    };
    assert_eq!(guest.status(), status, "budget={limit}");
    assert_eq!(guest.exception_code(), code, "budget={limit}");
    assert_eq!(guest.gas_limit(), limit);
    assert_eq!(guest.remaining_gas(), remaining);
    assert_eq!(guest.gas_used(), limit - remaining);
    let expected: Vec<_> = values.iter().copied().map(U256::from).collect();
    assert_eq!(guest.stack(), expected);
    assert_eq!(
        guest.final_s1(),
        EVM_STACK_REGION.start() + values.len() as u64 * EVM_WORD_SIZE
    );
}

#[test]
fn push1_boundaries_share_the_same_compiled_image() {
    use ferreth_compiler::{compiler::compile_evm_bytecode, orchestrator::execute_guest_push1};

    let code = [0x60, 2];
    let compiled = compile_evm_bytecode(&code).unwrap();
    for limit in [0, 1, 2, 3, 4, 1 << 63, u64::MAX] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_eq!(execution.assembly(), compiled.assembly());
        assert_eq!(execution.machine_code(), compiled.machine_code());
        if limit < 3 {
            assert_push_result(&execution, limit, 3, 0, &[]);
            assert_eq!(execution.instruction_metrics().opcode_body(), 6);
            assert_eq!(execution.instruction_metrics().guest_termination(), 10);
        } else {
            assert_push_result(&execution, limit, 0, limit - 3, &[2]);
            assert_eq!(execution.instruction_metrics().opcode_body(), 17);
            assert_eq!(execution.instruction_metrics().guest_termination(), 9);
        }
    }
    let isolated = execute_guest_push1(2).unwrap();
    assert_eq!(isolated.assembly(), compiled.assembly());
    assert_eq!(isolated.machine_code(), compiled.machine_code());
    assert_push_result(&isolated, u64::MAX, 0, u64::MAX - 3, &[2]);
}

#[test]
fn push1_charges_before_capacity_check() {
    let mut code = [0x60, 1].repeat(1024);
    code.extend([0x60, 2, 0]);
    for (limit, exception) in [(3074, 3), (3075, 1), (3080, 1)] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_push_result(&execution, limit, exception, 0, &[1; 1024]);
    }
}

#[test]
fn repeated_push1_preserves_successful_effects_on_later_oog() {
    let code = [0x60, 1, 0x60, 2, 0x60, 3];
    for (limit, exception, remaining, values, words) in [
        (10, 0, 1, &[1, 2, 3][..], 51),
        (9, 0, 0, &[1, 2, 3][..], 51),
        (8, 3, 0, &[1, 2][..], 40),
    ] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_push_result(&execution, limit, exception, remaining, values);
        assert_eq!(execution.instruction_metrics().opcode_body(), words);
    }
}

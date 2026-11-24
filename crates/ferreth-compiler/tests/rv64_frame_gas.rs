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

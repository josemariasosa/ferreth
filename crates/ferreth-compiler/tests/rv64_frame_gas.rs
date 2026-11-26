use std::process::Command;

use ferreth_compiler::{
    orchestrator::execute_guest_evm_bytecode,
    rv64::{guest::GuestExecutionStatus, memory::EVM_STACK_REGION},
};

fn assert_invalid_cli_hex(command: &str, bytecode: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_ferreth-compiler"))
        .args([command, "--bytecode", bytecode])
        .env("RUST_BACKTRACE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let evidence = format!(
        "{command} {bytecode:?}: {:?}\nstdout: {stdout:?}\nstderr: {stderr}",
        output.status.code()
    );
    assert_eq!(output.status.code(), Some(1), "{evidence}");
    assert!(
        stderr.contains("invalid bytecode hex:") && stderr.contains("hexadecimal"),
        "{evidence}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("backtrace"),
        "{evidence}"
    );
    assert!(stdout.is_empty(), "{evidence}");
}

#[test]
fn cli_execute_rejects_signed_hex() {
    assert_invalid_cli_hex("execute", "+1");
}

#[test]
fn cli_execute_rejects_utf8_hex_without_panicking() {
    assert_invalid_cli_hex("execute", "0é0");
}

#[test]
fn cli_decode_and_execute_reject_malformed_hex() {
    for command in ["decode", "execute"] {
        for bytecode in [
            "0", "0x00", "0X00", " 00 ", "00  00", "00\t00", "zz", "-1", "6+01",
        ] {
            assert_invalid_cli_hex(command, bytecode);
        }
    }
    for bytecode in ["+1", "0é0"] {
        assert_invalid_cli_hex("decode", bytecode);
    }
}

fn cli_stdout(command: &str, bytecode: &str, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ferreth-compiler"))
        .args([command, "--bytecode", bytecode])
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn cli_decode_and_execute_accept_case_insensitive_hex_and_empty_input() {
    for bytecode in ["60ab00", "60AB00", "60aB00"] {
        assert_eq!(
            cli_stdout("decode", bytecode, &[]),
            "PC      BYTES       OPCODE\n0x0000  60ab        PUSH1 0xab\n0x0002  00          STOP\n"
        );
        assert_eq!(
            cli_stdout("execute", bytecode, &["--gas", "4"]),
            "Decoded instructions: [Push(171), Stop]\nFinal status: Normal\nInitial frame gas: 4\nRemaining frame gas: 1\nGross gas used: 3\nFinal stack: [171]\nDynamic RV64 instruction count: 27\n"
        );
    }
    assert_eq!(
        cli_stdout("decode", "", &[]),
        "PC      BYTES       OPCODE\n"
    );
    assert_eq!(
        cli_stdout("execute", "", &["--gas", "0"]),
        "Decoded instructions: []\nFinal status: Normal\nInitial frame gas: 0\nRemaining frame gas: 0\nGross gas used: 0\nFinal stack: []\nDynamic RV64 instruction count: 9\n"
    );
}

#[test]
fn cli_distinguishes_valid_hex_with_decode_and_compilation_failures() {
    for (bytecode, diagnostic) in [
        ("02", "bytecode decoding failed: unsupported opcode 0x02 at byte offset 0"),
        ("610001", "bytecode execution failed: could not compile EVM bytecode: EVM opcode 0x61 at byte offset 0 is decoded but not supported by the RV64 compiler"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ferreth-compiler"))
            .args(["execute", "--bytecode", bytecode])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(String::from_utf8(output.stderr).unwrap(), format!("{diagnostic}\n"));
    }
}

#[test]
fn cli_reports_normal_and_exceptional_frame_results() {
    let normal = "Decoded instructions: [Gas, Stop]\nFinal status: Normal\nInitial frame gas: 2\nRemaining frame gas: 0\nGross gas used: 2\nFinal stack: [0]\nDynamic RV64 instruction count: 27\n";
    for budget in ["2", "0x2"] {
        assert_eq!(cli_stdout("execute", "5a00", &["--gas", budget]), normal);
    }
    for budget in ["0", "1"] {
        assert_eq!(
            cli_stdout("execute", "5a00", &["--gas", budget]),
            format!("Decoded instructions: [Gas, Stop]\nFinal status: GuestException\nGuest exception: OutOfGas (3)\nInitial frame gas: {budget}\nRemaining frame gas: 0\nGross gas used: {budget}\nFinal stack: []\nDynamic RV64 instruction count: 16\n")
        );
    }
    assert_eq!(
        cli_stdout("execute", "600101", &["--gas", "20"]),
        "Decoded instructions: [Push(1), Add]\nFinal status: GuestException\nGuest exception: StackUnderflow (2)\nInitial frame gas: 20\nRemaining frame gas: 0\nGross gas used: 20\nFinal stack: [1]\nDynamic RV64 instruction count: 37\n"
    );
}

#[test]
fn cli_reports_overflow_and_preserves_the_full_stack() {
    let bytecode = "6001".repeat(1024) + "5a00";
    let stdout = cli_stdout("execute", &bytecode, &["--gas", "3080"]);
    let (decoded, result) = stdout.split_once('\n').unwrap();
    assert!(decoded.starts_with("Decoded instructions: [Push(1), "));
    assert!(decoded.ends_with(", Gas, Stop]"));
    assert_eq!(
        result,
        format!("Final status: GuestException\nGuest exception: StackOverflow (1)\nInitial frame gas: 3080\nRemaining frame gas: 0\nGross gas used: 3080\nFinal stack: {:?}\nDynamic RV64 instruction count: 17429\n", [1; 1024])
    );
}

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
        assert_eq!(
            cli_stdout("execute", "00", args),
            format!("Decoded instructions: [Stop]\nFinal status: Normal\nInitial frame gas: {expected}\nRemaining frame gas: {expected}\nGross gas used: 0\nFinal stack: []\nDynamic RV64 instruction count: 10\n")
        );
    }
}

fn assert_frame_result(
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
fn gas_pushes_post_charge_u64_as_u256() {
    let code = [0x5a, 0];
    for limit in [0, 1, 2, 10, 1 << 63, u64::MAX] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        if limit < 2 {
            assert_frame_result(&execution, limit, 3, 0, &[]);
        } else {
            let remaining = limit - 2;
            assert_frame_result(&execution, limit, 0, remaining, &[remaining]);
            assert_eq!(
                execution.guest_result().stack()[0].as_limbs(),
                &[remaining, 0, 0, 0]
            );
        }
    }

    let execution = execute_guest_evm_bytecode(&[0x5a, 0x5a, 0], 10).unwrap();
    assert_frame_result(&execution, 10, 0, 6, &[8, 6]);

    let mut full = [0x60, 1].repeat(1024);
    full.extend([0x5a, 0]);
    for (limit, exception) in [(3073, 3), (3074, 1), (3080, 1)] {
        let execution = execute_guest_evm_bytecode(&full, limit).unwrap();
        assert_frame_result(&execution, limit, exception, 0, &[1; 1024]);
    }
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
            assert_frame_result(&execution, limit, 3, 0, &[]);
            assert_eq!(execution.instruction_metrics().opcode_body(), 6);
            assert_eq!(execution.instruction_metrics().guest_termination(), 10);
        } else {
            assert_frame_result(&execution, limit, 0, limit - 3, &[2]);
            assert_eq!(execution.instruction_metrics().opcode_body(), 17);
            assert_eq!(execution.instruction_metrics().guest_termination(), 9);
        }
    }
    let isolated = execute_guest_push1(2).unwrap();
    assert_eq!(isolated.assembly(), compiled.assembly());
    assert_eq!(isolated.machine_code(), compiled.machine_code());
    assert_frame_result(&isolated, u64::MAX, 0, u64::MAX - 3, &[2]);
}

#[test]
fn push1_charges_before_capacity_check() {
    let mut code = [0x60, 1].repeat(1024);
    code.extend([0x60, 2, 0]);
    for (limit, exception) in [(3074, 3), (3075, 1), (3080, 1)] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_frame_result(&execution, limit, exception, 0, &[1; 1024]);
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
        assert_frame_result(&execution, limit, exception, remaining, values);
        assert_eq!(execution.instruction_metrics().opcode_body(), words);
    }
}

#[test]
fn pop_charges_before_effects() {
    let code = [0x60, 2, 0x50, 0];
    for (limit, exception, remaining, stack) in
        [(4, 3, 0, &[2][..]), (5, 0, 0, &[][..]), (6, 0, 1, &[][..])]
    {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_frame_result(&execution, limit, exception, remaining, stack);
    }
}

#[test]
fn pop_charges_before_depth_validation() {
    for (limit, exception) in [(1, 3), (2, 2), (8, 2)] {
        let execution = execute_guest_evm_bytecode(&[0x50, 0], limit).unwrap();
        assert_frame_result(&execution, limit, exception, 0, &[]);
    }
}

#[test]
fn dup1_charges_before_effects() {
    let code = [0x60, 2, 0x80, 0];
    for (limit, exception, remaining, stack) in [
        (5, 3, 0, &[2][..]),
        (6, 0, 0, &[2, 2][..]),
        (7, 0, 1, &[2, 2][..]),
    ] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_frame_result(&execution, limit, exception, remaining, stack);
    }
}

#[test]
fn dup1_charges_before_depth_and_capacity_validation() {
    for (limit, exception) in [(2, 3), (3, 2), (8, 2)] {
        let execution = execute_guest_evm_bytecode(&[0x80, 0], limit).unwrap();
        assert_frame_result(&execution, limit, exception, 0, &[]);
    }
    let mut code = [0x60, 1].repeat(1024);
    code.extend([0x80, 0]);
    for (limit, exception) in [(3074, 3), (3075, 1), (3080, 1)] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_frame_result(&execution, limit, exception, 0, &[1; 1024]);
    }
}

#[test]
fn arithmetic_charges_before_effects() {
    for (opcode, result) in [(0x01, 5), (0x03, 1)] {
        let code = [0x60, 2, 0x60, 3, opcode, 0];
        for (limit, exception, remaining, stack) in [
            (8, 3, 0, &[2, 3][..]),
            (9, 0, 0, &[result][..]),
            (10, 0, 1, &[result][..]),
        ] {
            let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
            assert_frame_result(&execution, limit, exception, remaining, stack);
        }
    }
}

#[test]
fn arithmetic_charges_before_depth_validation() {
    for opcode in [0x01, 0x03] {
        for (limit, exception) in [(0, 3), (2, 3), (3, 2), (20, 2)] {
            let execution = execute_guest_evm_bytecode(&[opcode], limit).unwrap();
            assert_frame_result(&execution, limit, exception, 0, &[]);
        }
        for (limit, exception) in [(5, 3), (6, 2), (20, 2)] {
            let execution = execute_guest_evm_bytecode(&[0x60, 2, opcode, 0], limit).unwrap();
            assert_frame_result(&execution, limit, exception, 0, &[2]);
        }
    }
}

#[test]
fn mixed_program_charges_all_six_opcodes_in_one_budget_independent_image() {
    use ferreth_compiler::compiler::compile_evm_bytecode;

    let code = [0x60, 2, 0x80, 0x50, 0x60, 3, 0x01, 0x60, 7, 0x03, 0x00];
    let compiled = compile_evm_bytecode(&code).unwrap();
    for (limit, exception, remaining, stack) in [
        (19, 3, 0, &[5, 7][..]),
        (20, 0, 0, &[2][..]),
        (21, 0, 1, &[2][..]),
        (1 << 63, 0, (1 << 63) - 20, &[2][..]),
        (u64::MAX, 0, u64::MAX - 20, &[2][..]),
    ] {
        let execution = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_frame_result(&execution, limit, exception, remaining, stack);
        assert_eq!(execution.assembly(), compiled.assembly());
        assert_eq!(execution.machine_code(), compiled.machine_code());
    }
}

#[test]
fn every_new_charge_compares_unsigned_at_full_width() {
    use alloy_primitives::U256;
    use ferreth_compiler::orchestrator::execute_guest_evm_bytecode_with_initial_stack;

    for (opcode, cost, values) in [
        (0x50, 2, &[2][..]),
        (0x80, 3, &[2, 3, 3][..]),
        (0x01, 3, &[5][..]),
        (0x03, 3, &[1][..]),
    ] {
        let operands = [2, 3].map(U256::from);
        let at_cost =
            execute_guest_evm_bytecode_with_initial_stack(&[opcode], &operands, cost).unwrap();
        assert_frame_result(&at_cost, cost, 0, 0, values);
        for limit in [1 << 63, u64::MAX] {
            let result =
                execute_guest_evm_bytecode_with_initial_stack(&[opcode], &operands, limit).unwrap();
            assert_frame_result(&result, limit, 0, limit - cost, values);
            assert_eq!(result.machine_code(), at_cost.machine_code());
        }
    }
}

#[test]
fn gas_preserves_successful_prefix_when_next_gas_is_unfunded() {
    let execution = execute_guest_evm_bytecode(&[0x5a, 0x5a, 0], 3).unwrap();
    assert_frame_result(&execution, 3, 3, 0, &[1]);
}

#[test]
fn gas_image_and_cli_default_are_budget_independent() {
    use ferreth_compiler::compiler::compile_evm_bytecode;

    let code = [0x5a, 0];
    let compiled = compile_evm_bytecode(&code).unwrap();
    for limit in [0, 1, 2, 10, 1 << 63, u64::MAX] {
        let run = execute_guest_evm_bytecode(&code, limit).unwrap();
        assert_eq!(
            run.machine_code(),
            compiled.machine_code(),
            "budget={limit}"
        );
    }

    let run = execute_guest_evm_bytecode(&code, u64::MAX).unwrap();
    assert_frame_result(&run, u64::MAX, 0, u64::MAX - 2, &[u64::MAX - 2]);

    let explicit = u64::MAX.to_string();
    let default = cli_stdout("execute", "5a00", &[]);
    assert_eq!(
        default,
        cli_stdout("execute", "5a00", &["--gas", &explicit])
    );
    assert_eq!(
        default,
        cli_stdout("execute", "5a00", &["--gas", "0xffffffffffffffff"])
    );
    assert_eq!(
        default,
        "Decoded instructions: [Gas, Stop]\nFinal status: Normal\nInitial frame gas: 18446744073709551615\nRemaining frame gas: 18446744073709551613\nGross gas used: 2\nFinal stack: [18446744073709551613]\nDynamic RV64 instruction count: 27\n"
    );
}

#[test]
fn gas_decodes_with_offsets_and_requires_guest_execution() {
    use alloy_primitives::U256;
    use ferreth_compiler::{
        decode, decode_detailed, execution::ProgramRunner, DecodedInstruction, Instruction,
    };

    let code = [0x60, 0x5a, 0x5a, 0, 0x5a];
    assert_eq!(
        decode_detailed(&code).unwrap(),
        [
            DecodedInstruction::Push {
                value: U256::from(0x5a),
                opcode: 0x60,
                offset: 0,
            },
            DecodedInstruction::Gas { offset: 2 },
            DecodedInstruction::Stop { offset: 3 },
        ]
    );
    let ops = decode(&code).unwrap();
    assert_eq!(
        ops,
        [
            Instruction::Push(U256::from(0x5a)),
            Instruction::Gas,
            Instruction::Stop
        ]
    );
    assert_eq!(
        ProgramRunner::new().unwrap().run(&ops).unwrap_err(),
        "GAS requires the stack-aware RV64 guest execution path"
    );
}

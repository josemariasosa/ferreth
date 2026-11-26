use std::collections::BTreeSet;

use ferreth_compiler::{
    orchestrator::execute_guest_evm_bytecode,
    rv64::{
        guest::GuestExecutionStatus,
        memory::{EVM_STACK_REGION, EVM_WORD_SIZE},
    },
};
use revm::{
    bytecode::Bytecode,
    context_interface::host::DummyHost,
    interpreter::{
        InputsImpl, InstructionResult, Interpreter, InterpreterAction, SharedMemory,
        instruction_table,
        instructions::gas_table_spec,
        interpreter::{EthInterpreter, ExtBytecode},
        interpreter_types::Jumps,
    },
    primitives::{Bytes, U256, hardfork::SpecId},
};

struct RawReference<'a> {
    result: InstructionResult,
    remaining: u64,
    stack: &'a [u64],
}

#[derive(Debug, PartialEq, Eq)]
enum FrameOutcome<'a> {
    Normal(&'a [u64]),
    Exceptional,
}

struct SettledFrame<'a> {
    outcome: FrameOutcome<'a>,
    remaining: u64,
    gross_used: u64,
}

fn check(id: &str, code: &[u8], limit: u64, reference: RawReference<'_>, frame: SettledFrame<'_>) {
    let spec = SpecId::CANCUN;
    let mut interpreter = Interpreter::<EthInterpreter>::new(
        SharedMemory::new(),
        ExtBytecode::new(Bytecode::new_raw(Bytes::copy_from_slice(code))),
        InputsImpl::default(),
        false,
        spec,
        limit,
    );
    let action = interpreter.run_plain(
        &instruction_table::<EthInterpreter, DummyHost>(),
        &gas_table_spec(spec),
        &mut DummyHost::new(spec),
    );
    let InterpreterAction::Return(raw) = action else {
        panic!("{id} {action:?}")
    };
    let stack = interpreter.stack.data();
    eprintln!(
        "{id} REVM {:?}, remaining={}, used={}, stack={}, pc={}",
        raw.result,
        raw.gas.remaining(),
        limit - raw.gas.remaining(),
        diagnostic_stack(stack),
        interpreter.bytecode.pc()
    );

    assert_eq!(raw.result, reference.result, "{id} raw REVM result");
    assert_eq!(
        raw.gas.remaining(),
        reference.remaining,
        "{id} raw REVM remaining"
    );
    assert_eq!(*stack, words(reference.stack), "{id} raw REVM stack");

    // The pinned handler spends all remaining regular gas for exceptional
    // halts. This settlement is source-derived for V22. run_plain stops
    // before the handler, so preserve its raw observations above.
    let settled = match raw.result {
        InstructionResult::Stop => {
            let FrameOutcome::Normal(expected) = frame.outcome else {
                panic!("{id} expected exceptional reference termination")
            };
            assert_eq!(*stack, words(expected), "{id} successful REVM stack");
            raw.gas.remaining()
        }
        result if result.is_halt() => {
            assert_eq!(
                frame.outcome,
                FrameOutcome::Exceptional,
                "{id} reference outcome"
            );
            0
        }
        result => panic!("{id} unsupported reference termination {result:?}"),
    };
    assert_eq!(settled, frame.remaining, "{id} settled REVM remaining");
    assert_eq!(
        limit - settled,
        frame.gross_used,
        "{id} settled REVM gross used"
    );

    let execution = execute_guest_evm_bytecode(code, limit).unwrap_or_else(|e| panic!("{id} {e}"));
    let guest = execution.guest_result();
    eprintln!(
        "{id} Ferreth {:?}, code={}, remaining={}, used={}, stack={}",
        guest.status(),
        guest.exception_code(),
        guest.remaining_gas(),
        guest.gas_used(),
        diagnostic_stack(guest.stack())
    );
    match frame.outcome {
        FrameOutcome::Normal(expected) => {
            assert_eq!(guest.status(), GuestExecutionStatus::Normal, "{id}");
            assert_eq!(guest.stack(), words(expected), "{id}");
            assert_eq!(guest.exception_code(), 0, "{id}");
            assert_eq!(
                guest.final_s1(),
                EVM_STACK_REGION.start() + expected.len() as u64 * EVM_WORD_SIZE,
                "{id}"
            );
        }
        FrameOutcome::Exceptional => {
            assert_eq!(guest.status(), GuestExecutionStatus::GuestException, "{id}");
        }
    }
    assert_eq!(
        guest.remaining_gas(),
        frame.remaining,
        "{id} published remaining gas"
    );
    assert_eq!(guest.gas_limit(), limit, "{id}");
    assert_eq!(guest.gas_used(), frame.gross_used, "{id}");
}

fn words(values: &[u64]) -> Vec<U256> {
    values.iter().copied().map(U256::from).collect()
}

fn diagnostic_stack(stack: &[U256]) -> String {
    if stack.len() == 1024 && stack.iter().all(|&value| value == U256::from(1)) {
        "1024 ones".to_owned()
    } else {
        format!("{stack:?}")
    }
}

macro_rules! vectors {
    ($( $test:ident, $id:literal, $code:expr, $limit:literal;
     $raw_result:ident, $raw_remaining:literal, $raw_stack:expr;
     $outcome:expr, $settled:literal, $gross:literal; )+) => {
        #[test]
        fn every_corpus_id_runs_once() {
            let ids = [$($id),+];
            let expected: BTreeSet<_> = (1..=37).map(|number| format!("V{number:02}")).collect();
            let mut actual = BTreeSet::new();
            let mut duplicates = BTreeSet::new();
            for id in ids {
                if !actual.insert(id.to_owned()) {
                    duplicates.insert(id);
                }
            }
            let missing: Vec<_> = expected.difference(&actual).collect();
            let unexpected: Vec<_> = actual.difference(&expected).collect();
            assert!(
                ids.len() == expected.len()
                    && missing.is_empty()
                    && duplicates.is_empty()
                    && unexpected.is_empty(),
                "portable corpus has {} rows (expected {}); missing {missing:?}, duplicates {duplicates:?}, unexpected {unexpected:?}",
                ids.len(),
                expected.len(),
            );
        }

        $(
        #[test]
        fn $test() {
            check(
                $id,
                $code,
                $limit,
                RawReference {
                    result: InstructionResult::$raw_result,
                    remaining: $raw_remaining,
                    stack: $raw_stack,
                },
                SettledFrame {
                    outcome: $outcome,
                    remaining: $settled,
                    gross_used: $gross,
                },
            );
        }
    )+};
}

use FrameOutcome::{Exceptional, Normal};

fn full_stack_program(suffix: &[u8]) -> Vec<u8> {
    let mut code = [0x60, 1].repeat(1024);
    code.extend(suffix);
    code
}

// Raw and settled expectations are transcribed from the recorded corpus.
#[rustfmt::skip]
vectors! {
    // Test name, ID, bytecode, limit; raw outcome, remaining, stack; settled outcome, remaining, gross used.
    v01_stop, "V01", &[0x00], 0; Stop, 0, &[]; Normal(&[]), 0, 0;
    v02_stop_before_gas, "V02", &[0x00, 0x5a], 7; Stop, 7, &[]; Normal(&[]), 7, 0;
    v03_empty, "V03", &[], 0; Stop, 0, &[]; Normal(&[]), 0, 0;
    v04_push_below_cost, "V04", &[0x60, 0x02, 0x00], 2; OutOfGas, 0, &[]; Exceptional, 0, 2;
    v05_push_at_cost, "V05", &[0x60, 0x02, 0x00], 3; Stop, 0, &[2]; Normal(&[2]), 0, 3;
    v06_push_excess_at_eof, "V06", &[0x60, 0x02], 4; Stop, 1, &[2]; Normal(&[2]), 1, 3;
    v07_truncated_push, "V07", &[0x60], 3; Stop, 0, &[0]; Normal(&[0]), 0, 3;
    v08_pop_below_cost, "V08", &[0x60, 0x02, 0x50, 0x00], 4; OutOfGas, 0, &[2]; Exceptional, 0, 4;
    v09_pop_at_cost, "V09", &[0x60, 0x02, 0x50, 0x00], 5; Stop, 0, &[]; Normal(&[]), 0, 5;
    v10_pop_excess, "V10", &[0x60, 0x02, 0x50, 0x00], 6; Stop, 1, &[]; Normal(&[]), 1, 5;
    v11_pop_empty_below_cost, "V11", &[0x50, 0x00], 1; OutOfGas, 0, &[]; Exceptional, 0, 1;
    v12_dup1_below_cost, "V12", &[0x60, 0x02, 0x80, 0x00], 5; OutOfGas, 0, &[2]; Exceptional, 0, 5;
    v13_dup1_at_cost, "V13", &[0x60, 0x02, 0x80, 0x00], 6; Stop, 0, &[2, 2]; Normal(&[2, 2]), 0, 6;
    v14_dup1_excess, "V14", &[0x60, 0x02, 0x80, 0x00], 7; Stop, 1, &[2, 2]; Normal(&[2, 2]), 1, 6;
    v15_dup1_empty_at_cost, "V15", &[0x80, 0x00], 3; StackOverflow, 0, &[]; Exceptional, 0, 3;
    v16_add_below_cost, "V16", &[0x60, 0x02, 0x60, 0x03, 0x01, 0x00], 8; OutOfGas, 0, &[2, 3]; Exceptional, 0, 8;
    v17_add_at_cost, "V17", &[0x60, 0x02, 0x60, 0x03, 0x01, 0x00], 9; Stop, 0, &[5]; Normal(&[5]), 0, 9;
    v18_add_excess, "V18", &[0x60, 0x02, 0x60, 0x03, 0x01, 0x00], 10; Stop, 1, &[5]; Normal(&[5]), 1, 9;
    v19_add_empty_zero_gas, "V19", &[0x01], 0; OutOfGas, 0, &[]; Exceptional, 0, 0;
    v20_add_empty_below_cost, "V20", &[0x01], 2; OutOfGas, 0, &[]; Exceptional, 0, 2;
    v21_add_empty_at_cost, "V21", &[0x01], 3; StackUnderflow, 0, &[]; Exceptional, 0, 3;
    v22_add_one_operand, "V22", &[0x60, 0x02, 0x01, 0x00], 20; StackUnderflow, 14, &[2]; Exceptional, 0, 20;
    v23_sub_below_cost, "V23", &[0x60, 0x02, 0x60, 0x03, 0x03, 0x00], 8; OutOfGas, 0, &[2, 3]; Exceptional, 0, 8;
    v24_sub_at_cost, "V24", &[0x60, 0x02, 0x60, 0x03, 0x03, 0x00], 9; Stop, 0, &[1]; Normal(&[1]), 0, 9;
    v25_sub_excess, "V25", &[0x60, 0x02, 0x60, 0x03, 0x03, 0x00], 10; Stop, 1, &[1]; Normal(&[1]), 1, 9;
    v26_gas_zero_budget, "V26", &[0x5a, 0x00], 0; OutOfGas, 0, &[]; Exceptional, 0, 0;
    v27_gas_below_cost, "V27", &[0x5a, 0x00], 1; OutOfGas, 0, &[]; Exceptional, 0, 1;
    v28_gas_at_cost, "V28", &[0x5a, 0x00], 2; Stop, 0, &[0]; Normal(&[0]), 0, 2;
    v29_repeated_gas, "V29", &[0x5a, 0x5a, 0x00], 10; Stop, 6, &[8, 6]; Normal(&[8, 6]), 6, 4;
    v30_gas_max_budget, "V30", &[0x5a, 0x00], 18446744073709551615; Stop, 18446744073709551613, &[18446744073709551613]; Normal(&[18446744073709551613]), 18446744073709551613, 2;
    v31_gas_unsigned_boundary, "V31", &[0x5a, 0x00], 9223372036854775808; Stop, 9223372036854775806, &[9223372036854775806]; Normal(&[9223372036854775806]), 9223372036854775806, 2;
    v32_full_stack_below_cost, "V32", &full_stack_program(&[0x60, 2, 0]), 3074; OutOfGas, 0, &[1; 1024]; Exceptional, 0, 3074;
    v33_full_stack_at_cost, "V33", &full_stack_program(&[0x60, 2, 0]), 3075; StackOverflow, 0, &[1; 1024]; Exceptional, 0, 3075;
    v34_full_stack_gas_below_cost, "V34", &full_stack_program(&[0x5a, 0]), 3073; OutOfGas, 0, &[1; 1024]; Exceptional, 0, 3073;
    v35_full_stack_gas_at_cost, "V35", &full_stack_program(&[0x5a, 0]), 3074; StackOverflow, 0, &[1; 1024]; Exceptional, 0, 3074;
    v36_full_stack_dup1, "V36", &full_stack_program(&[0x80, 0]), 3075; StackOverflow, 0, &[1; 1024]; Exceptional, 0, 3075;
    v37_mixed_gas_at_eof, "V37", &[0x60, 0x02, 0x80, 0x01, 0x5a, 0x50, 0x60, 0x07, 0x03, 0x5a], 25; Stop, 4, &[3, 4]; Normal(&[3, 4]), 4, 21;
}

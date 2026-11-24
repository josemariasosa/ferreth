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

fn check(
    id: &str,
    code: &[u8],
    limit: u64,
    expected: InstructionResult,
    remaining: u64,
    expected_stack: &[U256],
) {
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
        "{id} REVM {:?}, remaining={}, used={}, stack={stack:?}, pc={}",
        raw.result,
        raw.gas.remaining(),
        limit - raw.gas.remaining(),
        interpreter.bytecode.pc()
    );
    assert_eq!(raw.result, expected, "{id} raw REVM result");
    assert_eq!(raw.gas.remaining(), remaining, "{id} raw REVM remaining");
    assert_eq!(stack, expected_stack, "{id} raw REVM stack");
    // These rows already report zero raw gas on exceptions. Use the documented
    // exceptional frame settlement for reference comparisons only.
    let normal = raw.result == InstructionResult::Stop;
    let settled = if normal { raw.gas.remaining() } else { 0 };
    assert_eq!(settled, remaining, "{id} settled REVM remaining");
    let execution = execute_guest_evm_bytecode(code, limit).unwrap_or_else(|e| panic!("{id} {e}"));
    let guest = execution.guest_result();
    eprintln!(
        "{id} Ferreth {:?}, code={}, remaining={}, used={}, depth={}",
        guest.status(),
        guest.exception_code(),
        guest.remaining_gas(),
        guest.gas_used(),
        guest.stack().len()
    );
    let status = if normal {
        GuestExecutionStatus::Normal
    } else {
        GuestExecutionStatus::GuestException
    };
    assert_eq!(guest.status(), status, "{id}");
    if normal {
        assert_eq!(guest.stack(), expected_stack, "{id}");
        assert_eq!(guest.exception_code(), 0, "{id}");
        assert_eq!(
            guest.final_s1(),
            EVM_STACK_REGION.start() + expected_stack.len() as u64 * EVM_WORD_SIZE,
            "{id}"
        );
    }
    assert_eq!(
        guest.remaining_gas(),
        remaining,
        "{id} published remaining gas"
    );
    assert_eq!(guest.gas_limit(), limit, "{id}");
    assert_eq!(guest.gas_used(), limit - remaining, "{id}");
}

#[test]
fn v01_stop() {
    check("V01", &[0], 0, InstructionResult::Stop, 0, &[]);
}

#[test]
fn v02_stop_before_gas() {
    check("V02", &[0, 0x5a], 7, InstructionResult::Stop, 7, &[]);
}

#[test]
fn v03_empty() {
    check("V03", &[], 0, InstructionResult::Stop, 0, &[]);
}

#[test]
fn v04_push_below_cost() {
    check("V04", &[0x60, 2, 0], 2, InstructionResult::OutOfGas, 0, &[]);
}

#[test]
fn v05_push_at_cost() {
    check(
        "V05",
        &[0x60, 2, 0],
        3,
        InstructionResult::Stop,
        0,
        &[U256::from(2)],
    );
}

#[test]
fn v06_push_excess_at_eof() {
    check(
        "V06",
        &[0x60, 2],
        4,
        InstructionResult::Stop,
        1,
        &[U256::from(2)],
    );
}

#[test]
fn v07_truncated_push() {
    check("V07", &[0x60], 3, InstructionResult::Stop, 0, &[U256::ZERO]);
}

fn check_full_stack(id: &str, limit: u64, expected: InstructionResult) {
    let mut code = [0x60, 1].repeat(1024);
    code.extend([0x60, 2, 0]);
    check(id, &code, limit, expected, 0, &vec![U256::from(1); 1024]);
}

#[test]
fn v32_full_stack_below_cost() {
    check_full_stack("V32", 3074, InstructionResult::OutOfGas);
}

#[test]
fn v33_full_stack_at_cost() {
    check_full_stack("V33", 3075, InstructionResult::StackOverflow);
}

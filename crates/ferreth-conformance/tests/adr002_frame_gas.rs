use ferreth_compiler::{
    orchestrator::execute_guest_evm_bytecode,
    rv64::{guest::GuestExecutionStatus, memory::EVM_STACK_REGION},
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
    primitives::{Bytes, hardfork::SpecId},
};

fn check(id: &str, code: &[u8], limit: u64) {
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
    assert_eq!(raw.result, InstructionResult::Stop, "{id}");
    assert_eq!(raw.gas.remaining(), limit, "{id}");
    assert!(stack.is_empty(), "{id}");
    let execution = execute_guest_evm_bytecode(code, limit).unwrap_or_else(|e| panic!("{id} {e}"));
    let guest = execution.guest_result();
    assert_eq!(guest.status(), GuestExecutionStatus::Normal, "{id}");
    assert_eq!(guest.exception_code(), 0, "{id}");
    assert!(guest.stack().is_empty(), "{id}");
    assert_eq!(guest.final_s1(), EVM_STACK_REGION.start(), "{id}");
    assert_eq!(guest.remaining_gas(), limit, "{id} published remaining gas");
    assert_eq!(guest.gas_limit(), limit, "{id}");
    assert_eq!(guest.gas_used(), 0, "{id}");
}

#[test]
fn v02_stop_before_gas() {
    check("V02", &[0, 0x5a], 7);
}

#[test]
fn v01_stop() {
    check("V01", &[0], 0);
}

#[test]
fn v03_empty() {
    check("V03", &[], 0);
}

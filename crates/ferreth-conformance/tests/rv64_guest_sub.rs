use ferreth_compiler::{
    Opcode,
    rv64::{
        executor::{IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{initialize_guest_state, read_evm_word, write_evm_word},
        memory::{ENTRY_PC, EVM_STACK_REGION, EXIT_PC, validate_generated_code},
        opcodes::sub::generate_guest_sub,
    },
};
use ferreth_conformance::run_evm;
use revm::primitives::U256;

const NATIVE_SP_SENTINEL: u64 = 0xbfff_f000;

fn jal_x0(relative_offset: u32) -> u32 {
    assert_eq!(
        relative_offset & 1,
        0,
        "JAL offsets must be two-byte aligned"
    );
    assert!(
        relative_offset < (1 << 20),
        "test jump must be in JAL range"
    );
    ((relative_offset & 0x7fe) << 20)
        | ((relative_offset & 0x800) << 9)
        | (relative_offset & 0xff000)
        | 0x6f
}

/// Appends a test-only normal exit after the SUB body's continuation label.
fn program_with_exit() -> Vec<u8> {
    let generated = generate_guest_sub();
    let mut program = generated.machine_code().to_vec();
    let jump_pc = ENTRY_PC + u64::try_from(program.len()).expect("program length fits u64");
    program.extend_from_slice(&jal_x0((EXIT_PC - jump_pc) as u32).to_le_bytes());
    validate_generated_code(ENTRY_PC, program.len()).expect("SUB test program fits code region");
    program
}

fn initialized_executor(initial_stack: &[U256]) -> RvemuExecutor {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    executor
        .load_code(&program_with_exit())
        .expect("load SUB test program");
    executor.write_register(IntegerRegister::STACK_POINTER, NATIVE_SP_SENTINEL);
    initialize_guest_state(&mut executor).expect("initialize guest state");
    for (slot, word) in initial_stack.iter().copied().enumerate() {
        write_evm_word(&mut executor, slot, word).expect("seed live EVM stack word");
    }
    executor.write_register(
        IntegerRegister::EVM_STACK_NEXT_FREE,
        EVM_STACK_REGION.start() + initial_stack.len() as u64 * 32,
    );
    executor
}

fn execute_sub(initial_stack: &[U256]) -> RvemuExecutor {
    let mut executor = initialized_executor(initial_stack);
    executor
        .execute(ENTRY_PC, EXIT_PC, 100)
        .expect("valid guest SUB reaches test-only exit epilogue");
    executor
}

#[test]
fn sub_uses_evm_top_first_order_confirmed_by_revm() {
    assert_eq!(
        run_evm(Opcode::Sub, U256::from(3), U256::from(5)).expect("revm executes SUB"),
        U256::MAX - U256::from(1),
        "revm confirms EVM stack [5, 3] evaluates 3 - 5"
    );
    assert_eq!(
        read_evm_word(&mut execute_sub(&[U256::from(5), U256::from(3)]), 0).expect("read result"),
        U256::MAX - U256::from(1)
    );
    assert_eq!(
        read_evm_word(&mut execute_sub(&[U256::from(3), U256::from(5)]), 0).expect("read result"),
        U256::from(2)
    );
    assert_eq!(
        read_evm_word(&mut execute_sub(&[U256::from(5), U256::from(5)]), 0).expect("read result"),
        U256::ZERO
    );
}

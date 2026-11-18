use asm_rs::{assemble, Arch};
use ferreth_compiler::{
    add_u256,
    rv64::{
        executor::{ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{initialize_guest_state, read_evm_word, write_evm_word},
        memory::{validate_generated_code, ENTRY_PC, EVM_STACK_REGION, EXIT_PC},
        opcodes::add::generate_guest_add,
    },
};
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

/// Appends a test-only normal exit after the ADD body's continuation label.
fn program_with_exit() -> Vec<u8> {
    let generated = generate_guest_add();
    let mut program = generated.machine_code().to_vec();
    let jump_pc = ENTRY_PC + u64::try_from(program.len()).expect("program length fits u64");
    program.extend_from_slice(&jal_x0((EXIT_PC - jump_pc) as u32).to_le_bytes());
    validate_generated_code(ENTRY_PC, program.len()).expect("ADD test program fits code region");
    program
}

fn initialized_executor(initial_stack: &[U256]) -> RvemuExecutor {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    executor
        .load_code(&program_with_exit())
        .expect("load ADD test program");
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

fn execute_add(initial_stack: &[U256]) -> RvemuExecutor {
    let mut executor = initialized_executor(initial_stack);
    executor
        .execute(ENTRY_PC, EXIT_PC, 100)
        .expect("valid guest ADD reaches test-only exit epilogue");
    executor
}

#[test]
fn add_five_and_three_executes_entirely_on_the_guest_stack() {
    let executor = execute_add(&[U256::from(5), U256::from(3)]);
    assert_eq!(
        read_evm_word(&mut { executor }, 0).expect("read result"),
        U256::from(8)
    );
}

#[test]
fn add_propagates_carry_across_u64_limbs_and_wraps_at_u256() {
    let carry_a = U256::from_limbs([u64::MAX, u64::MAX, 0, 0]);
    let carry_b = U256::from(1);
    let carry = execute_add(&[carry_a, carry_b]);
    assert_eq!(
        read_evm_word(&mut { carry }, 0).expect("read carry result"),
        U256::from_limbs([0, 0, 1, 0])
    );

    let overflow = execute_add(&[U256::MAX, U256::from(1)]);
    assert_eq!(
        read_evm_word(&mut { overflow }, 0).expect("read wrapped result"),
        U256::ZERO
    );
}

#[test]
fn add_aliases_lower_operand_retires_one_slot_and_preserves_abi_registers() {
    let lower = U256::from_limbs([7, 11, 13, 17]);
    let top = U256::from_limbs([19, 23, 29, 31]);
    let mut executor = initialized_executor(&[lower, top]);
    let initial_s0 = executor.read_register(IntegerRegister::EVM_STACK_BASE);
    let top_slot_address = EVM_STACK_REGION.start() + 32;
    let before_top = read_evm_word(&mut executor, 1).expect("read top input");
    executor
        .execute(ENTRY_PC, EXIT_PC, 100)
        .expect("ADD reaches test-only exit epilogue");
    assert_eq!(
        read_evm_word(&mut executor, 0).expect("read lower alias destination"),
        U256::from_limbs(add_u256(lower.into_limbs(), top.into_limbs()))
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        EVM_STACK_REGION.start() + 32,
        "ADD decreases depth by exactly one"
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        initial_s0
    );
    assert_eq!(
        executor.read_register(IntegerRegister::STACK_POINTER),
        NATIVE_SP_SENTINEL
    );
    assert_eq!(
        read_evm_word(&mut executor, 1).expect("read inactive top slot"),
        before_top,
        "the top operand becomes inactive without being cleared"
    );
    assert_eq!(top_slot_address, EVM_STACK_REGION.start() + 32);
}

#[test]
fn add_underflow_at_depth_zero_or_one_preserves_stack_and_registers() {
    for initial_stack in [vec![], vec![U256::from(9)]] {
        let mut executor = initialized_executor(&initial_stack);
        let before_s0 = executor.read_register(IntegerRegister::EVM_STACK_BASE);
        let before_s1 = executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE);
        let before_sp = executor.read_register(IntegerRegister::STACK_POINTER);
        let before_stack = initial_stack
            .iter()
            .enumerate()
            .map(|(slot, _)| read_evm_word(&mut executor, slot).expect("read seeded stack"))
            .collect::<Vec<_>>();
        assert_eq!(
            executor.execute(ENTRY_PC, EXIT_PC, 5),
            Err(ExecutorError::InstructionLimitExceeded {
                limit: 5,
                executed_instructions: 5,
            })
        );
        assert_eq!(
            executor.read_register(IntegerRegister::EVM_STACK_BASE),
            before_s0
        );
        assert_eq!(
            executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
            before_s1
        );
        assert_eq!(
            executor.read_register(IntegerRegister::STACK_POINTER),
            before_sp
        );
        let after_stack = initial_stack
            .iter()
            .enumerate()
            .map(|(slot, _)| read_evm_word(&mut executor, slot).expect("read retained stack"))
            .collect::<Vec<_>>();
        assert_eq!(after_stack, before_stack);
    }
}

#[test]
fn add_at_maximum_depth_is_valid_and_retires_the_top_slot() {
    let mut executor = initialized_executor(&[]);
    write_evm_word(&mut executor, 1022, U256::from(5)).expect("seed lower full-stack operand");
    write_evm_word(&mut executor, 1023, U256::from(3)).expect("seed top full-stack operand");
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, EVM_STACK_REGION.end());

    executor
        .execute(ENTRY_PC, EXIT_PC, 100)
        .expect("ADD at full architectural depth reaches the exit epilogue");
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        EVM_STACK_REGION.end() - 32
    );
    assert_eq!(
        read_evm_word(&mut executor, 1022).expect("read full-stack result"),
        U256::from(8)
    );
}

#[test]
fn add_machine_code_matches_assembly_and_exposes_link_points() {
    let generated = generate_guest_add();
    assert_eq!(
        assemble(generated.assembly(), Arch::Rv64).expect("reassemble readable ADD wrapper"),
        generated.machine_code()
    );
    let links = generated
        .link_metadata()
        .expect("ADD exposes link metadata");
    assert_eq!(links.core_call_offset(), 20);
    assert_eq!(links.exceptional_trampoline_offset(), 32);
    assert_eq!(
        links.normal_continuation_offset(),
        generated.machine_code().len()
    );
    assert_eq!(
        u32::from_le_bytes(
            generated.machine_code()[links.exceptional_trampoline_offset()..][..4]
                .try_into()
                .expect("trampoline is one instruction")
        ),
        0x0000_006f
    );
}

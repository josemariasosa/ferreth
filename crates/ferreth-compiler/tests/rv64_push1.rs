use asm_rs::{assemble, Arch};
use ferreth_compiler::rv64::{
    executor::{ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor},
    guest::initialize_guest_state,
    memory::{validate_generated_code, ENTRY_PC, EVM_STACK_REGION, EXIT_PC},
    opcodes::push::generate_guest_push1,
};

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

/// Appends a test-only successful-exit epilogue. It is not part of PUSH1.
fn program_with_exit(immediate: u8) -> Vec<u8> {
    let generated = generate_guest_push1(immediate);
    let mut program = generated.machine_code().to_vec();
    let jump_pc = ENTRY_PC + u64::try_from(program.len()).expect("program length fits u64");
    program.extend_from_slice(&jal_x0((EXIT_PC - jump_pc) as u32).to_le_bytes());
    validate_generated_code(ENTRY_PC, program.len()).expect("PUSH1 test program fits code region");
    program
}

fn initialized_executor(immediate: u8) -> RvemuExecutor {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    executor
        .load_code(&program_with_exit(immediate))
        .expect("load PUSH1 test program");
    executor.write_register(IntegerRegister::STACK_POINTER, NATIVE_SP_SENTINEL);
    initialize_guest_state(&mut executor, 0).expect("initialize guest state");
    executor
}

#[test]
fn push1_writes_zero_five_and_maximum_byte_as_little_endian_u256_words() {
    for immediate in [0, 5, u8::MAX] {
        let mut executor = initialized_executor(immediate);
        let stack_base = EVM_STACK_REGION.start();

        let report = executor
            .execute(ENTRY_PC, EXIT_PC, 11)
            .expect("successful PUSH1 reaches test-only exit epilogue");

        assert_eq!(report.executed_instructions, 11, "PUSH1 0x{immediate:02x}");
        assert_eq!(
            executor.read_register(IntegerRegister::EVM_STACK_BASE),
            stack_base,
            "PUSH1 0x{immediate:02x} preserves s0"
        );
        assert_eq!(
            executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
            stack_base + 32,
            "PUSH1 0x{immediate:02x} advances s1 by exactly one word"
        );
        assert_eq!(
            executor.read_register(IntegerRegister::STACK_POINTER),
            NATIVE_SP_SENTINEL,
            "PUSH1 0x{immediate:02x} preserves native sp"
        );

        let mut word = [0; 32];
        executor
            .read_memory(stack_base, &mut word)
            .expect("read PUSH1 stack word");
        let mut expected = [0; 32];
        expected[0] = immediate;
        assert_eq!(word, expected, "PUSH1 0x{immediate:02x}");
    }
}

#[test]
fn push1_overflow_leaves_stack_and_abi_registers_unchanged() {
    let immediate = 5;
    let mut executor = initialized_executor(immediate);
    let stack_base = EVM_STACK_REGION.start();
    let stack_limit = EVM_STACK_REGION.end();
    let before = vec![0xa5; EVM_STACK_REGION.size() as usize];
    executor
        .write_memory(stack_base, &before)
        .expect("seed complete EVM stack");
    let beyond_stack = [0x5a; 32];
    executor
        .write_memory(stack_limit, &beyond_stack)
        .expect("seed stack-adjacent sentinel");
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, stack_limit);

    assert_eq!(
        executor.execute(ENTRY_PC, EXIT_PC, 5),
        Err(ExecutorError::InstructionLimitExceeded {
            limit: 5,
            executed_instructions: 5,
        }),
        "the patchable overflow trampoline must not report successful completion"
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        stack_base,
        "overflow preserves s0"
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        stack_limit,
        "overflow must not advance s1"
    );
    assert_eq!(
        executor.read_register(IntegerRegister::STACK_POINTER),
        NATIVE_SP_SENTINEL,
        "overflow preserves native sp"
    );
    let mut after = vec![0; before.len()];
    executor
        .read_memory(stack_base, &mut after)
        .expect("read EVM stack after overflow");
    assert_eq!(after, before, "overflow must not modify the EVM stack");
    let mut beyond_stack_after = [0; 32];
    executor
        .read_memory(stack_limit, &mut beyond_stack_after)
        .expect("read stack-adjacent sentinel after overflow");
    assert_eq!(
        beyond_stack_after, beyond_stack,
        "overflow must not write beyond the EVM stack"
    );
}

#[test]
fn push1_machine_code_is_assembled_from_its_readable_assembly() {
    let generated = generate_guest_push1(5);
    assert_eq!(
        assemble(generated.assembly(), Arch::Rv64).expect("reassemble readable PUSH1 assembly"),
        generated.machine_code(),
    );
    assert_eq!(generated.overflow_trampoline_offset(), 40);
    assert_eq!(
        generated.instruction_words().collect::<Vec<_>>(),
        vec![
            0x0000_82b7, // lui t0, 8
            0x0054_02b3, // add t0, s0, t0
            0x0254_f063, // bgeu s1, t0, 32
            0x0050_0293, // addi t0, zero, 5
            0x0054_b023, // sd t0, 0(s1)
            0x0004_b423, // sd zero, 8(s1)
            0x0004_b823, // sd zero, 16(s1)
            0x0004_bc23, // sd zero, 24(s1)
            0x0204_8493, // addi s1, s1, 32
            0x0080_006f, // jal zero, 8
            0x0000_006f, // patchable jal zero, 0
        ]
    );
}

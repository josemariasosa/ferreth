use alloy_primitives::U256;
use ferreth_compiler::{
    execution::Rv64Runner,
    rv64::{
        executor::{ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor},
        memory::{validate_generated_code, LayoutError, ENTRY_PC, EXIT_PC},
    },
    Opcode,
};

const ADDI_X5_X0_42: u32 = 0x02a0_0293;
const JAL_X0_PLUS_4: u32 = 0x0040_006f;
const JAL_X0_SELF: u32 = 0x0000_006f;
const EBREAK: u32 = 0x0010_0073;

fn program(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn jal_x0(relative_offset: u32) -> u32 {
    assert_eq!(
        relative_offset & 1,
        0,
        "JAL offsets must be two-byte aligned"
    );
    assert!(
        relative_offset < (1 << 20),
        "test jump must be forward and in range"
    );
    ((relative_offset & 0x7fe) << 20)
        | ((relative_offset & 0x800) << 9)
        | (relative_offset & 0xff000)
        | 0x6f
}

#[test]
fn executes_loaded_program_and_stops_before_exit_sentinel() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    executor
        .load_code(&program(&[ADDI_X5_X0_42, JAL_X0_PLUS_4, EBREAK]))
        .expect("load RV64 program");

    let report = executor
        .execute(RvemuExecutor::DRAM_BASE, RvemuExecutor::DRAM_BASE + 8, 3)
        .expect("execute through configured exit PC");

    assert_eq!(report.executed_instructions, 2);
    assert_eq!(
        executor.read_register(IntegerRegister::new(5).unwrap()),
        42,
        "the exit EBREAK must not execute"
    );
}

#[test]
fn execute_until_stops_at_a_measured_publisher_boundary_before_fetching_it() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    let boundary = RvemuExecutor::DRAM_BASE + 4;
    executor
        .load_code(&program(&[ADDI_X5_X0_42, EBREAK]))
        .expect("load RV64 program");

    let body = executor
        .execute_until(RvemuExecutor::DRAM_BASE, &[boundary], 2)
        .expect("stop at boundary");
    assert_eq!(body.executed_instructions, 1);
    assert_eq!(body.exit_pc, boundary);
    assert_eq!(executor.read_register(IntegerRegister::new(5).unwrap()), 42);
}

#[test]
fn guest_jumps_to_the_reserved_exit_pc_before_it_is_fetched() {
    let machine_code = program(&[ADDI_X5_X0_42, jal_x0((EXIT_PC - (ENTRY_PC + 4)) as u32)]);
    validate_generated_code(ENTRY_PC, machine_code.len())
        .expect("guest code must leave the reserved exit slot unoccupied");

    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    executor.load_code(&machine_code).expect("load guest code");
    // A breakpoint at the exit slot turns an accidental execution of that
    // address into an execution fault. It is not generated guest code.
    executor
        .write_memory(EXIT_PC, &EBREAK.to_le_bytes())
        .expect("place exit-slot trap marker");

    let report = executor
        .execute(ENTRY_PC, EXIT_PC, 2)
        .expect("jump to the accepted exit boundary");

    assert_eq!(report.executed_instructions, 2);
    assert_eq!(
        executor.read_register(IntegerRegister::new(5).unwrap()),
        42,
        "the observable guest operation must complete before the exit jump"
    );
}

#[test]
fn generated_code_cannot_overlap_the_reserved_exit_slot() {
    assert!(matches!(
        validate_generated_code(ENTRY_PC, (EXIT_PC - ENTRY_PC + 4) as usize),
        Err(LayoutError::CodeOverlapsExitSlot { .. })
    ));
}

#[test]
fn registers_and_guest_memory_round_trip() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    let register = IntegerRegister::new(10).unwrap();
    executor.write_register(register, 0x0123_4567_89ab_cdef);
    assert_eq!(executor.read_register(register), 0x0123_4567_89ab_cdef);

    let address = RvemuExecutor::DRAM_BASE + 0x2000;
    let expected = [0x00, 0x12, 0x7f, 0xff];
    executor.write_memory(address, &expected).unwrap();
    let mut actual = [0; 4];
    executor.read_memory(address, &mut actual).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn instruction_limit_is_distinct_from_normal_completion() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    let machine_code = program(&[JAL_X0_SELF]);
    validate_generated_code(ENTRY_PC, machine_code.len()).unwrap();
    executor.load_code(&machine_code).unwrap();

    assert_eq!(
        executor.execute(ENTRY_PC, EXIT_PC, 2),
        Err(ExecutorError::InstructionLimitExceeded {
            limit: 2,
            executed_instructions: 2,
        })
    );
}

#[test]
fn execution_fault_is_not_normal_completion() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    let machine_code = program(&[EBREAK]);
    validate_generated_code(ENTRY_PC, machine_code.len()).unwrap();
    executor.load_code(&machine_code).unwrap();

    assert!(matches!(
        executor.execute(ENTRY_PC, EXIT_PC, 1),
        Err(ExecutorError::ExecutionFault {
            executed_instructions: 0,
            ..
        })
    ));
}

#[test]
fn arithmetic_runner_retains_its_ebreak_return_sentinel_behavior() {
    let execution = Rv64Runner::new(Opcode::Add)
        .and_then(|mut runner| runner.run(U256::from(5), U256::from(7)))
        .expect("execute historical ADD runner");

    assert_eq!(execution.result, U256::from(12));
    assert_eq!(
        execution.metrics.total, 34,
        "the appended EBREAK return sentinel remains excluded from the historical count"
    );
}

#[test]
fn rejects_out_of_range_host_memory_accesses_including_crossing_width() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    assert!(matches!(
        executor.write_memory(RvemuExecutor::DRAM_END, &[0]),
        Err(ExecutorError::MemoryOutOfRange { .. })
    ));
    assert!(matches!(
        executor.read_memory(RvemuExecutor::DRAM_END - 1, &mut [0; 2]),
        Err(ExecutorError::MemoryOutOfRange { .. })
    ));
}

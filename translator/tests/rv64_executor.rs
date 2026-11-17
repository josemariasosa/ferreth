use ferrum_eth_translator::rv64::executor::{
    ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor,
};

const ADDI_X5_X0_42: u32 = 0x02a0_0293;
const JAL_X0_PLUS_4: u32 = 0x0040_006f;
const JAL_X0_SELF: u32 = 0x0000_006f;
const EBREAK: u32 = 0x0010_0073;

fn program(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
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
    executor.load_code(&program(&[JAL_X0_SELF])).unwrap();

    assert_eq!(
        executor.execute(RvemuExecutor::DRAM_BASE, RvemuExecutor::DRAM_BASE + 4, 2),
        Err(ExecutorError::InstructionLimitExceeded {
            limit: 2,
            executed_instructions: 2,
        })
    );
}

#[test]
fn execution_fault_is_not_normal_completion() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    executor.load_code(&program(&[EBREAK])).unwrap();

    assert!(matches!(
        executor.execute(RvemuExecutor::DRAM_BASE, RvemuExecutor::DRAM_BASE + 4, 1),
        Err(ExecutorError::ExecutionFault { .. })
    ));
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

use ferrum_eth_translator::rv64::{
    executor::{ExecutionReport, ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor},
    guest::{
        initialize_guest_state, read_guest_result, GuestExecutionStatus, GuestMetadata,
        GuestMetadataError, GuestResultError, EVM_WORD_ENCODED_SIZE, GUEST_METADATA_ENCODED_SIZE,
    },
    memory::{validate_generated_code, ENTRY_PC, EVM_STACK_REGION, EXIT_PC, METADATA_REGION},
};
use revm::primitives::U256;

const EBREAK: u32 = 0x0010_0073;

fn program(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn addi(rd: u8, rs1: u8, immediate: i16) -> u32 {
    assert!(matches!(immediate, -2048..=2047));
    ((immediate as u16 as u32) << 20) | (u32::from(rs1) << 15) | (u32::from(rd) << 7) | 0x13
}

fn sd(rs2: u8, rs1: u8, immediate: i16) -> u32 {
    assert!(matches!(immediate, -2048..=2047));
    let immediate = immediate as u16 as u32;
    ((immediate & 0xfe0) << 20)
        | (u32::from(rs2) << 20)
        | (u32::from(rs1) << 15)
        | (0b011 << 12)
        | ((immediate & 0x1f) << 7)
        | 0x23
}

fn append_exit_jump(words: &mut Vec<u32>) {
    let jump_pc = ENTRY_PC + u64::try_from(words.len()).expect("test program length fits u64") * 4;
    let offset = EXIT_PC - jump_pc;
    assert!(offset < 1 << 20, "test jump must be in JAL range");
    assert_eq!(offset & 1, 0, "JAL offset must be two-byte aligned");
    let offset = offset as u32;
    words.push(((offset & 0x7fe) << 20) | ((offset & 0x800) << 9) | (offset & 0xff000) | 0x6f);
}

fn metadata_pointer(words: &mut Vec<u32>) {
    // s0 starts at 0x8010_1000. Two legal ADDIs derive metadata at
    // 0x8010_0000 without requiring a sign-extended 32-bit address constant.
    words.push(addi(6, 8, -2048));
    words.push(addi(6, 6, -2048));
}

fn normal_one_word_program() -> Vec<u32> {
    let mut words = vec![
        addi(5, 0, 5),
        sd(5, 9, 0),
        sd(0, 9, 8),
        sd(0, 9, 16),
        sd(0, 9, 24),
        addi(9, 9, 32),
    ];
    metadata_pointer(&mut words);
    words.extend([
        sd(9, 6, 16),
        sd(0, 6, 8),
        addi(5, 0, GuestExecutionStatus::Normal.as_u64() as i16),
        // Status is the final metadata store, signalling publication.
        sd(5, 6, 0),
    ]);
    append_exit_jump(&mut words);
    words
}

fn normal_empty_stack_program() -> Vec<u32> {
    let mut words = Vec::new();
    metadata_pointer(&mut words);
    words.extend([
        sd(9, 6, 16),
        sd(0, 6, 8),
        addi(5, 0, GuestExecutionStatus::Normal.as_u64() as i16),
        sd(5, 6, 0),
    ]);
    append_exit_jump(&mut words);
    words
}

fn exceptional_empty_stack_program(exception_code: i16) -> Vec<u32> {
    let mut words = Vec::new();
    metadata_pointer(&mut words);
    words.extend([
        sd(9, 6, 16),
        addi(5, 0, exception_code),
        sd(5, 6, 8),
        addi(5, 0, GuestExecutionStatus::GuestException.as_u64() as i16),
        sd(5, 6, 0),
    ]);
    append_exit_jump(&mut words);
    words
}

fn exit_only_program() -> Vec<u32> {
    let mut words = Vec::new();
    append_exit_jump(&mut words);
    words
}

fn initialized_executor_with_code(words: &[u32]) -> RvemuExecutor {
    let machine_code = program(words);
    validate_generated_code(ENTRY_PC, machine_code.len()).expect("valid test guest code");
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    executor.load_code(&machine_code).expect("load guest code");
    initialize_guest_state(&mut executor).expect("initialize guest state");
    // An accidental fetch of the exit PC would fault. The executor must
    // instead observe it before fetch and return the execution report.
    executor
        .write_memory(EXIT_PC, &EBREAK.to_le_bytes())
        .expect("mark exit slot as unexecutable");
    executor
}

#[test]
fn rv64_publishes_and_host_retrieves_a_normal_one_word_result() {
    let mut executor = initialized_executor_with_code(&normal_one_word_program());
    let report = executor
        .execute(ENTRY_PC, EXIT_PC, 13)
        .expect("guest jumps to deliberate exit boundary");

    assert_eq!(report.executed_instructions, 13);
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        EVM_STACK_REGION.start() + 32
    );

    let result = read_guest_result(&mut executor, &report).expect("retrieve published result");
    assert_eq!(result.status(), GuestExecutionStatus::Normal);
    assert_eq!(result.exception_code(), 0);
    assert_eq!(result.final_s1(), EVM_STACK_REGION.start() + 32);
    assert_eq!(result.stack_depth(), 1);
    assert_eq!(result.stack(), &[U256::from(5)]);

    let mut word_bytes = [0; EVM_WORD_ENCODED_SIZE];
    executor
        .read_memory(EVM_STACK_REGION.start(), &mut word_bytes)
        .expect("read RV64-written EVM word");
    let mut expected_word_bytes = [0; EVM_WORD_ENCODED_SIZE];
    expected_word_bytes[0] = 5;
    assert_eq!(word_bytes, expected_word_bytes);

    let mut metadata_bytes = [0; GUEST_METADATA_ENCODED_SIZE];
    executor
        .read_memory(METADATA_REGION.start(), &mut metadata_bytes)
        .expect("read RV64-published metadata");
    let mut expected_metadata_bytes = [0; GUEST_METADATA_ENCODED_SIZE];
    expected_metadata_bytes[..8].copy_from_slice(&1_u64.to_le_bytes());
    expected_metadata_bytes[16..].copy_from_slice(&(EVM_STACK_REGION.start() + 32).to_le_bytes());
    assert_eq!(
        metadata_bytes, expected_metadata_bytes,
        "RV64 must publish status, exception code, and final s1 at the accepted ABI offsets"
    );
}

#[test]
fn rv64_can_publish_a_normal_empty_stack_result() {
    let mut executor = initialized_executor_with_code(&normal_empty_stack_program());
    let report = executor
        .execute(ENTRY_PC, EXIT_PC, 7)
        .expect("guest jumps to deliberate exit boundary");

    assert_eq!(report.executed_instructions, 7);
    let result = read_guest_result(&mut executor, &report).expect("retrieve empty result");
    assert_eq!(result.status(), GuestExecutionStatus::Normal);
    assert_eq!(result.stack_depth(), 0);
    assert!(result.stack().is_empty());
}

#[test]
fn rv64_published_guest_exception_is_not_an_emulator_failure() {
    let mut executor = initialized_executor_with_code(&exceptional_empty_stack_program(42));
    let report = executor
        .execute(ENTRY_PC, EXIT_PC, 8)
        .expect("guest exception still terminates at deliberate exit boundary");

    assert_eq!(report.executed_instructions, 8);
    let result = read_guest_result(&mut executor, &report).expect("retrieve exceptional result");
    assert_eq!(result.status(), GuestExecutionStatus::GuestException);
    assert_eq!(result.exception_code(), 42);
    assert_eq!(result.stack_depth(), 0);
    assert!(result.stack().is_empty());
}

fn completed_executor_for_negative_metadata() -> (RvemuExecutor, ExecutionReport) {
    let mut executor = initialized_executor_with_code(&exit_only_program());
    let report = executor
        .execute(ENTRY_PC, EXIT_PC, 1)
        .expect("exit-only test program completes deliberately");
    (executor, report)
}

#[test]
fn retrieval_rejects_invalid_metadata_and_live_s1_mismatch() {
    let cases = [
        (
            "unknown status",
            {
                let mut bytes =
                    GuestMetadata::new(GuestExecutionStatus::Normal, 0, EVM_STACK_REGION.start())
                        .encode();
                bytes[..8].copy_from_slice(&99_u64.to_le_bytes());
                bytes
            },
            GuestResultError::Metadata(GuestMetadataError::InvalidStatus { value: 99 }),
        ),
        (
            "still running",
            GuestMetadata::new(GuestExecutionStatus::Running, 0, EVM_STACK_REGION.start()).encode(),
            GuestResultError::Metadata(GuestMetadataError::StillRunningOrIncomplete),
        ),
        (
            "misaligned final stack pointer",
            GuestMetadata::new(
                GuestExecutionStatus::Normal,
                0,
                EVM_STACK_REGION.start() + 1,
            )
            .encode(),
            GuestResultError::Metadata(GuestMetadataError::InvalidFinalStackPointer(
                ferrum_eth_translator::rv64::memory::LayoutError::StackPointerMisaligned {
                    next_free: EVM_STACK_REGION.start() + 1,
                },
            )),
        ),
        (
            "final stack pointer below base",
            GuestMetadata::new(
                GuestExecutionStatus::Normal,
                0,
                EVM_STACK_REGION.start() - 32,
            )
            .encode(),
            GuestResultError::Metadata(GuestMetadataError::InvalidFinalStackPointer(
                ferrum_eth_translator::rv64::memory::LayoutError::StackPointerOutOfRange {
                    next_free: EVM_STACK_REGION.start() - 32,
                },
            )),
        ),
        (
            "final stack pointer beyond limit",
            GuestMetadata::new(GuestExecutionStatus::Normal, 0, EVM_STACK_REGION.end() + 32)
                .encode(),
            GuestResultError::Metadata(GuestMetadataError::InvalidFinalStackPointer(
                ferrum_eth_translator::rv64::memory::LayoutError::StackPointerOutOfRange {
                    next_free: EVM_STACK_REGION.end() + 32,
                },
            )),
        ),
        (
            "normal status with exception",
            GuestMetadata::new(GuestExecutionStatus::Normal, 1, EVM_STACK_REGION.start()).encode(),
            GuestResultError::Metadata(GuestMetadataError::NormalStatusHasExceptionCode {
                exception_code: 1,
            }),
        ),
    ];

    for (label, metadata, expected) in cases {
        let (mut executor, report) = completed_executor_for_negative_metadata();
        assert_eq!(report.executed_instructions, 1, "{label}");
        executor
            .write_memory(METADATA_REGION.start(), &metadata)
            .expect("host writes only negative-test metadata");
        assert_eq!(
            read_guest_result(&mut executor, &report),
            Err(expected),
            "{label}"
        );
    }

    let (mut executor, report) = completed_executor_for_negative_metadata();
    executor
        .write_memory(
            METADATA_REGION.start(),
            &GuestMetadata::new(
                GuestExecutionStatus::Normal,
                0,
                EVM_STACK_REGION.start() + 32,
            )
            .encode(),
        )
        .expect("write negative-test metadata");
    assert_eq!(
        read_guest_result(&mut executor, &report),
        Err(GuestResultError::LiveStackPointerMismatch {
            published_s1: EVM_STACK_REGION.start() + 32,
            live_s1: EVM_STACK_REGION.start(),
        })
    );
}

#[test]
fn retrieval_rejects_a_changed_live_stack_base() {
    let (mut executor, report) = completed_executor_for_negative_metadata();
    executor
        .write_memory(
            METADATA_REGION.start(),
            &GuestMetadata::new(GuestExecutionStatus::Normal, 0, EVM_STACK_REGION.start()).encode(),
        )
        .expect("publish otherwise-valid test metadata");
    let changed_s0 = EVM_STACK_REGION.start() + 32;
    executor.write_register(IntegerRegister::EVM_STACK_BASE, changed_s0);

    assert_eq!(
        read_guest_result(&mut executor, &report),
        Err(GuestResultError::LiveStackBaseMismatch {
            expected_s0: EVM_STACK_REGION.start(),
            live_s0: changed_s0,
        })
    );
}

#[test]
fn failed_execution_cannot_produce_a_guest_result() {
    let mut executor = initialized_executor_with_code(&[0x0000_006f]); // jal x0, 0
    assert_eq!(
        executor.execute(ENTRY_PC, EXIT_PC, 2),
        Err(ExecutorError::InstructionLimitExceeded {
            limit: 2,
            executed_instructions: 2,
        })
    );

    let mut executor = initialized_executor_with_code(&[EBREAK]);
    assert!(matches!(
        executor.execute(ENTRY_PC, EXIT_PC, 1),
        Err(ExecutorError::ExecutionFault {
            executed_instructions: 0,
            ..
        })
    ));
    // Both failures return no ExecutionReport. `read_guest_result` requires
    // that report, so neither failed path can be converted into a result.
}

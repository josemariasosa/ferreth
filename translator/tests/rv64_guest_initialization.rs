use ferrum_eth_translator::rv64::{
    executor::{IntegerRegister, Rv64Executor, RvemuExecutor},
    guest::{
        initialize_guest_state, GuestExecutionStatus, GuestMetadata, GUEST_METADATA_ENCODED_SIZE,
    },
    memory::{evm_stack_depth, EVM_STACK_REGION, METADATA_REGION},
};

fn initialized_executor() -> RvemuExecutor {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    initialize_guest_state(&mut executor).expect("initialize EVM guest state");
    executor
}

#[test]
fn initialization_sets_empty_stack_registers_and_exact_running_metadata() {
    let mut executor = initialized_executor();
    let stack_base = EVM_STACK_REGION.start();

    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        stack_base
    );
    let s1 = executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE);
    assert_eq!(s1, stack_base);
    assert_eq!(evm_stack_depth(s1), Ok(0));

    let mut actual = [0; GUEST_METADATA_ENCODED_SIZE];
    executor
        .read_memory(METADATA_REGION.start(), &mut actual)
        .expect("read initial metadata");
    let expected = GuestMetadata::new(GuestExecutionStatus::Running, 0, stack_base);
    assert_eq!(actual, expected.encode());

    let decoded = GuestMetadata::decode(&actual).expect("decode initial metadata");
    assert_eq!(decoded.status(), GuestExecutionStatus::Running);
    assert_eq!(decoded.exception_code(), 0);
    assert_eq!(decoded.final_s1(), stack_base);
}

#[test]
fn initialization_preserves_unrelated_native_sp() {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    let original_sp = 0xdead_beef_cafe_0000;
    executor.write_register(IntegerRegister::STACK_POINTER, original_sp);

    initialize_guest_state(&mut executor).expect("initialize EVM guest state");

    assert_eq!(
        executor.read_register(IntegerRegister::STACK_POINTER),
        original_sp,
        "the current runtime has no native-stack requirement"
    );
}

#[test]
fn reinitialization_discards_logical_stack_liveness_and_restores_abi_state() {
    let mut executor = initialized_executor();
    let stack_base = EVM_STACK_REGION.start();

    // Simulate a reused executor: this stale slot must not become live after
    // initialization, and the initializer must not need to inspect or clear it.
    executor
        .write_memory(stack_base, &[0xa5; 32])
        .expect("write stale stack slot");
    executor.write_register(IntegerRegister::EVM_STACK_BASE, stack_base + 32);
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, stack_base + 64);
    executor
        .write_memory(
            METADATA_REGION.start(),
            &[0xff; GUEST_METADATA_ENCODED_SIZE],
        )
        .expect("mutate metadata");

    initialize_guest_state(&mut executor).expect("reinitialize EVM guest state");

    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        stack_base
    );
    let s1 = executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE);
    assert_eq!(s1, stack_base);
    assert_eq!(evm_stack_depth(s1), Ok(0));

    let mut metadata = [0; GUEST_METADATA_ENCODED_SIZE];
    executor
        .read_memory(METADATA_REGION.start(), &mut metadata)
        .expect("read reset metadata");
    assert_eq!(
        GuestMetadata::decode(&metadata),
        Ok(GuestMetadata::new(
            GuestExecutionStatus::Running,
            0,
            stack_base
        ))
    );
}

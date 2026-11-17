use ferreth_compiler::rv64::{
    executor::{IntegerRegister, Rv64Executor, RvemuExecutor},
    guest::{
        initialize_guest_state, read_evm_word, write_evm_word, GuestMemoryError, GuestMetadata,
        EVM_WORD_ENCODED_SIZE, GUEST_METADATA_ENCODED_SIZE,
    },
    memory::{EVM_STACK_CAPACITY, EVM_STACK_REGION, METADATA_REGION},
};
use revm::primitives::U256;

fn initialized_executor() -> RvemuExecutor {
    let mut executor = RvemuExecutor::new().expect("initialize rvemu executor");
    initialize_guest_state(&mut executor).expect("initialize EVM guest state");
    executor
}

#[test]
fn words_round_trip_across_all_four_limbs() {
    let cases = [
        U256::ZERO,
        U256::from(1),
        U256::from(u64::MAX),
        U256::from_limbs([0, 1, 0, 0]),
        U256::from_limbs([0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210, 1, u64::MAX]),
        U256::MAX,
        U256::from_limbs([
            0xaaaa_aaaa_aaaa_aaaa,
            0x5555_5555_5555_5555,
            0xaaaa_aaaa_aaaa_aaaa,
            0x5555_5555_5555_5555,
        ]),
        // Fixed deterministic pseudo-random vector covering all four limbs.
        U256::from_limbs([
            0x9e37_79b9_7f4a_7c15,
            0xbf58_476d_1ce4_e5b9,
            0x94d0_49bb_1331_11eb,
            0xd2b7_4407_b1ce_6e93,
        ]),
    ];
    let mut executor = initialized_executor();

    for (slot_index, word) in cases.into_iter().enumerate() {
        write_evm_word(&mut executor, slot_index, word).expect("write EVM word");
        assert_eq!(
            read_evm_word(&mut executor, slot_index).expect("read EVM word"),
            word,
            "slot {slot_index}"
        );
    }
}

#[test]
fn first_slot_has_exact_little_endian_limb_encoding() {
    let mut executor = initialized_executor();
    let limbs = [
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210,
        0x0f1e_2d3c_4b5a_6978,
        0x8877_6655_4433_2211,
    ];
    let word = U256::from_limbs(limbs);

    write_evm_word(&mut executor, 0, word).expect("write first slot");

    let mut actual = [0; EVM_WORD_ENCODED_SIZE];
    executor
        .read_memory(EVM_STACK_REGION.start(), &mut actual)
        .expect("read first slot bytes");
    let expected = [
        0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01, // limb 0
        0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, // limb 1
        0x78, 0x69, 0x5a, 0x4b, 0x3c, 0x2d, 0x1e, 0x0f, // limb 2
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, // limb 3
    ];
    assert_eq!(actual, expected);
}

#[test]
fn last_stack_slot_is_valid() {
    let mut executor = initialized_executor();
    let word = U256::from_limbs([1, 2, 3, 4]);
    let slot_index = EVM_STACK_CAPACITY as usize - 1;

    write_evm_word(&mut executor, slot_index, word).expect("write last slot");
    assert_eq!(
        read_evm_word(&mut executor, slot_index).expect("read last slot"),
        word
    );
}

#[test]
fn invalid_stack_slots_are_rejected() {
    let mut executor = initialized_executor();
    for slot_index in [
        EVM_STACK_CAPACITY as usize,
        EVM_STACK_CAPACITY as usize + 1,
        usize::MAX,
    ] {
        assert_eq!(
            write_evm_word(&mut executor, slot_index, U256::ZERO),
            Err(GuestMemoryError::InvalidStackSlot { slot_index })
        );
        assert_eq!(
            read_evm_word(&mut executor, slot_index),
            Err(GuestMemoryError::InvalidStackSlot { slot_index })
        );
    }
}

#[test]
fn word_io_preserves_stack_registers_and_metadata() {
    let mut executor = initialized_executor();
    let original_s0 = executor.read_register(IntegerRegister::EVM_STACK_BASE);
    let original_s1 = executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE);
    let mut original_metadata = [0; GUEST_METADATA_ENCODED_SIZE];
    executor
        .read_memory(METADATA_REGION.start(), &mut original_metadata)
        .expect("read metadata before word I/O");

    write_evm_word(&mut executor, 17, U256::from_limbs([1, 2, 3, 4])).expect("write EVM word");
    let _ = read_evm_word(&mut executor, 17).expect("read EVM word");

    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        original_s0
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        original_s1
    );
    let mut actual_metadata = [0; GUEST_METADATA_ENCODED_SIZE];
    executor
        .read_memory(METADATA_REGION.start(), &mut actual_metadata)
        .expect("read metadata after word I/O");
    assert_eq!(actual_metadata, original_metadata);
    assert_eq!(
        GuestMetadata::decode(&actual_metadata),
        GuestMetadata::decode(&original_metadata)
    );
}

#[test]
fn writing_a_slot_overwrites_its_prior_word() {
    let mut executor = initialized_executor();
    let first = U256::from_limbs([1, 2, 3, 4]);
    let replacement = U256::from_limbs([u64::MAX, 0, u64::MAX, 0]);

    write_evm_word(&mut executor, 29, first).expect("write initial word");
    write_evm_word(&mut executor, 29, replacement).expect("overwrite word");

    assert_eq!(
        read_evm_word(&mut executor, 29).expect("read overwritten word"),
        replacement
    );
}

#[test]
fn repeated_word_reads_and_writes_are_deterministic() {
    let mut executor = initialized_executor();
    let words = [
        U256::from_limbs([0x1111_1111_1111_1111, 0, 0, 0]),
        U256::from_limbs([0, 0x2222_2222_2222_2222, 0, 0]),
        U256::from_limbs([0, 0, 0x3333_3333_3333_3333, 0]),
        U256::from_limbs([0, 0, 0, 0x4444_4444_4444_4444]),
    ];

    for _ in 0..3 {
        for (slot_index, word) in words.into_iter().enumerate() {
            write_evm_word(&mut executor, slot_index, word).expect("write deterministic word");
            assert_eq!(
                read_evm_word(&mut executor, slot_index).expect("read deterministic word"),
                word
            );
        }
    }
}

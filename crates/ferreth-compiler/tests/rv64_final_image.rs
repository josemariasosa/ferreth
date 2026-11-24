use alloy_primitives::U256;
use ferreth_compiler::{
    compiler::{compile_evm_bytecode, CompiledRv64Program},
    orchestrator::{
        execute_guest_add_with_initial_stack, execute_guest_evm_bytecode,
        execute_guest_sub_with_initial_stack,
    },
    rv64::{guest::GuestExecutionStatus, memory::ENTRY_PC},
};

#[derive(Clone, Copy, Debug)]
enum Publisher {
    Normal,
    Underflow,
    Overflow,
    OutOfGas,
}

use Publisher::{Normal, OutOfGas, Overflow, Underflow};

// Independent ADR-001 encodings for the pointer, exception code, status and exit.
fn assert_publishers(code: &[u8], pcs: &[u64], mut offset: usize, roles: &[Publisher]) {
    assert_eq!(pcs.len(), roles.len(), "publisher inventory length");
    for (&pc, &role) in pcs.iter().zip(roles) {
        assert_eq!(
            pc,
            ENTRY_PC + offset as u64,
            "ordered {role:?} publisher PC"
        );
        let prefix = [0x8004_0313_u32, 0x8003_0313, 0x0093_3823];
        let stores: &[u32] = match role {
            Normal => &[
                0x0003_3423,
                0x0203_3283,
                0x0253_3423,
                0x0010_0293,
                0x0053_3023,
            ],
            Underflow | Overflow | OutOfGas => &[
                match role {
                    Overflow => 0x0010_0293,
                    Underflow => 0x0020_0293,
                    OutOfGas => 0x0030_0293,
                    Normal => unreachable!(),
                },
                0x0053_3423,
                0x0203_3023,
                0x0203_3423,
                0x0020_0293,
                0x0053_3023,
            ],
        };
        let end = offset + 4 * (prefix.len() + stores.len());
        let words: Vec<_> = code[offset..end]
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_le_bytes)
            .collect();
        let expected: Vec<_> = prefix.into_iter().chain(stores.iter().copied()).collect();
        assert_eq!(words, expected, "{role:?} publisher body");
        assert_eq!(
            code[end..end + 4],
            0xffc3_0067_u32.to_le_bytes(),
            "{role:?} terminal JALR"
        );
        offset = end + 4;
    }
    assert_eq!(offset, code.len(), "complete publisher tail");
}

#[test]
fn verified_push_only_and_mixed_images_keep_public_artifacts_and_execution() {
    let bytecode = [0x60, 2];
    let push = compile_evm_bytecode(&bytecode).unwrap();
    assert_eq!(
        push.termination_entry_pcs(),
        [ENTRY_PC + 76, ENTRY_PC + 112, ENTRY_PC + 152]
    );
    assert_publishers(
        push.machine_code(),
        push.termination_entry_pcs(),
        76,
        &[Normal, Overflow, OutOfGas],
    );
    // The first branch skips one word. The OOG JAL at byte 20 reaches
    // byte 152, a displacement of 132. The debit updates metadata +32.
    let charge_words = [
        0x8004_0313_u32,
        0x8003_0313,
        0x0203_3283,
        0x0030_0393,
        0x0072_f463,
        0x0840_006f,
        0x4072_82b3,
        0x0253_3023,
    ];
    assert_eq!(
        &push.machine_code()[..32],
        charge_words.map(u32::to_le_bytes).as_flattened()
    );
    let executed = execute_guest_evm_bytecode(&bytecode, u64::MAX).unwrap();
    assert_eq!(executed.machine_code(), push.machine_code());
    assert_eq!(executed.assembly(), push.assembly());
    assert_eq!(executed.instruction_metrics().total(), 26);

    let bytecode = [0x60, 2, 0x80, 0x50, 0x60, 3, 0x01, 0x60, 7, 0x03, 0x00];
    let mixed = compile_evm_bytecode(&bytecode).unwrap();
    assert_eq!(
        mixed.termination_entry_pcs(),
        [
            ENTRY_PC + 784,
            ENTRY_PC + 820,
            ENTRY_PC + 860,
            ENTRY_PC + 900
        ]
    );
    assert_eq!(mixed.machine_code().len(), 940);
    assert_publishers(
        mixed.machine_code(),
        mixed.termination_entry_pcs(),
        784,
        &[Normal, Underflow, Overflow, OutOfGas],
    );
    let executed = execute_guest_evm_bytecode(&bytecode, u64::MAX).unwrap();
    assert_eq!(executed.machine_code(), mixed.machine_code());
    assert_eq!(executed.assembly(), mixed.assembly());
    assert_eq!(
        executed.guest_result().status(),
        GuestExecutionStatus::Normal
    );
    assert_eq!(executed.guest_result().stack(), &[U256::from(2)]);
    assert_eq!(executed.instruction_metrics().opcode_body(), 181);
    assert_eq!(executed.instruction_metrics().guest_termination(), 9);
    assert_eq!(executed.instruction_metrics().total(), 190);
}

#[test]
fn verified_isolated_arithmetic_keeps_three_publishers_for_both_paths() {
    let operands = [U256::from(2), U256::from(3)];
    for (opcode, execute, expected) in [
        (
            0x01,
            execute_guest_add_with_initial_stack as fn(&[U256]) -> _,
            5,
        ),
        (0x03, execute_guest_sub_with_initial_stack, 1),
    ] {
        let normal = execute(&operands).unwrap();
        let underflow = execute(&[]).unwrap();
        assert_eq!(normal.machine_code(), underflow.machine_code());
        assert_eq!(normal.assembly(), underflow.assembly());
        assert_eq!(normal.machine_code().len(), 320);
        assert_eq!(normal.assembly().matches("jalr zero, -4(t1)").count(), 3);
        let compiled = compile_evm_bytecode(&[opcode]).unwrap();
        assert_publishers(
            normal.machine_code(),
            compiled.termination_entry_pcs(),
            204,
            &[Normal, Underflow, OutOfGas],
        );
        assert_eq!(normal.guest_result().status(), GuestExecutionStatus::Normal);
        assert_eq!(normal.guest_result().stack(), &[U256::from(expected)]);
        assert_eq!(normal.instruction_metrics().total(), 58);
        assert_eq!(normal.guest_result().remaining_gas(), u64::MAX - 3);
        assert_eq!(normal.guest_result().gas_used(), 3);
        assert_eq!(
            underflow.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(underflow.guest_result().exception_code(), 2);
        assert_eq!(underflow.instruction_metrics().total(), 20);
    }
}

#[test]
fn verified_images_without_exception_publishers_end_after_normal() {
    for (bytecode, offset) in [(&[][..], 0), (&[0x00][..], 4)] {
        let compiled = compile_evm_bytecode(bytecode).unwrap();
        assert_publishers(
            compiled.machine_code(),
            compiled.termination_entry_pcs(),
            offset,
            &[Normal],
        );
    }
}

fn both_publisher_fixture() -> CompiledRv64Program {
    let compiled = compile_evm_bytecode(&[0x80]).unwrap();
    assert_publishers(
        compiled.machine_code(),
        compiled.termination_entry_pcs(),
        96,
        &[Normal, Underflow, Overflow, OutOfGas],
    );
    compiled
}

#[test]
#[should_panic(expected = "Underflow publisher body")]
fn publisher_check_rejects_swapped_exception_bodies_with_unchanged_pcs() {
    let compiled = both_publisher_fixture();
    let mut bytes = compiled.machine_code().to_vec();
    let (underflow, overflow) = bytes[132..212].split_at_mut(40);
    underflow.swap_with_slice(overflow);
    assert_publishers(
        &bytes,
        compiled.termination_entry_pcs(),
        96,
        &[Normal, Underflow, Overflow, OutOfGas],
    );
}

#[test]
#[should_panic(expected = "Overflow terminal JALR")]
fn publisher_check_rejects_an_incorrect_terminal_jalr() {
    let compiled = both_publisher_fixture();
    let mut bytes = compiled.machine_code().to_vec();
    // jalr zero, 0(t1) lands at metadata instead of the exit sentinel.
    bytes[208..212].copy_from_slice(&0x0003_0067_u32.to_le_bytes());
    assert_publishers(
        &bytes,
        compiled.termination_entry_pcs(),
        96,
        &[Normal, Underflow, Overflow, OutOfGas],
    );
}

#[test]
#[should_panic(expected = "ordered Underflow publisher PC")]
fn publisher_check_rejects_reordered_metadata() {
    let compiled = both_publisher_fixture();
    let mut pcs = compiled.termination_entry_pcs().to_vec();
    pcs.swap(1, 2);
    assert_publishers(
        compiled.machine_code(),
        &pcs,
        96,
        &[Normal, Underflow, Overflow, OutOfGas],
    );
}

#[test]
#[should_panic(expected = "ordered Overflow publisher PC")]
fn publisher_check_rejects_duplicate_metadata() {
    let compiled = both_publisher_fixture();
    let mut pcs = compiled.termination_entry_pcs().to_vec();
    pcs[2] = pcs[1];
    assert_publishers(
        compiled.machine_code(),
        &pcs,
        96,
        &[Normal, Underflow, Overflow, OutOfGas],
    );
}

#[test]
#[should_panic(expected = "Normal publisher body")]
fn publisher_check_rejects_omitted_gas_store() {
    let compiled = compile_evm_bytecode(&[]).unwrap();
    let mut bytes = compiled.machine_code().to_vec();
    bytes[20..24].copy_from_slice(&0x0000_0013_u32.to_le_bytes());
    assert_publishers(&bytes, compiled.termination_entry_pcs(), 0, &[Normal]);
}

#[test]
#[should_panic(expected = "Normal publisher body")]
fn publisher_check_rejects_status_before_gas() {
    let compiled = compile_evm_bytecode(&[]).unwrap();
    let mut bytes = compiled.machine_code().to_vec();
    // Move status setup and store ahead of the load/store of remaining gas.
    bytes[16..32].rotate_left(8);
    assert_publishers(&bytes, compiled.termination_entry_pcs(), 0, &[Normal]);
}

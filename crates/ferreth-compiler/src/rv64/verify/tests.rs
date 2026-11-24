use super::*;
use crate::compiler::{compile_evm_bytecode, compile_evm_bytecode_legacy};
use asm_rs::{assemble, Arch};

const NOP: u32 = 0x0000_0013;
const MIXED_INPUT: &[u8] = &[0x60, 7, 0x80, 0x50, 0x01, 0x03, 0x00];
const MIXED_FRAGMENTS: [Fragment; 6] = [
    Fragment::Push1,
    Fragment::Dup1,
    Fragment::Pop,
    Fragment::Arithmetic,
    Fragment::Arithmetic,
    Fragment::Stop,
];

fn check_words(words: &[u32], sites: &[Site]) -> Result<(), String> {
    let bytes = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    Image {
        bytes: &bytes,
        base_pc: ENTRY_PC,
    }
    .verify_block(ENTRY_PC, words.len() as u64, sites)
}

#[test]
fn decodes_signed_jal_literals() {
    for (word, rd, displacement) in [
        (0x0080_006f, 0, 8),
        (0xffdf_f06f, 0, -4),
        (0x0090_006f, 0, 2056),
        (0x8000_006f, 0, -1_048_576),
        (0x7fff_f06f, 0, 1_048_574),
        (0x7fdf_f06f, 0, 1_048_572),
        (0x0080_00ef, 1, 8),
    ] {
        assert_eq!(
            decode_transfer(ENTRY_PC, word),
            Ok((Transfer::Jal { rd }, displacement))
        );
    }
    check_words(&[NOP, 0xffdf_f06f], &[Site::jal(1, 0, ENTRY_PC)]).unwrap();
}

#[test]
fn decodes_signed_branch_literals() {
    for (word, funct3, rs1, rs2, displacement) in [
        (0x0062_8463, 0, 5, 6, 8),
        (0xfe62_8ee3, 0, 5, 6, -4),
        (0x0062_80e3, 0, 5, 6, 2048),
        (0x8000_0063, 0, 0, 0, -4096),
        (0x7e00_0fe3, 0, 0, 0, 4094),
    ] {
        assert_eq!(
            decode_transfer(ENTRY_PC, word),
            Ok((Transfer::Branch { funct3, rs1, rs2 }, displacement))
        );
    }
    check_words(&[NOP, 0xfe62_8ee3], &[Site::branch(1, 0, 5, 6, ENTRY_PC)]).unwrap();
}

#[test]
fn rejects_wrong_jal_target_register_and_opcode() {
    let sites = [Site::jal(0, 0, ENTRY_PC + 8)];
    check_words(&[0x0080_006f, NOP, NOP, NOP], &sites).unwrap();
    for (word, diagnostic) in [
        (0x00c0_006f, "targets 0x8000000c, expected 0x80000008"),
        (0x0080_00ef, "Jal { rd: 1 }, expected Jal { rd: 0 }"),
        (NOP, "unexpected opcode 0x13"),
        (0x0062_8463, "expected Jal { rd: 0 }"),
    ] {
        let error = check_words(&[word, NOP, NOP, NOP], &sites).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn rejects_lf027_missing_jal_sign_bit() {
    let sites = [Site::jal(1, 0, ENTRY_PC)];
    let correct = 0xffdf_f06f;
    check_words(&[NOP, correct], &sites).unwrap();
    let error = check_words(&[NOP, correct ^ 0x8000_0000], &sites).unwrap_err();
    assert!(
        error.contains("full word at PC 0x80100000 is outside"),
        "{error}"
    );
}

#[test]
fn rejects_wrong_branch_condition_operands_and_target() {
    let sites = [Site::branch(0, 0, 5, 6, ENTRY_PC + 8)];
    check_words(&[0x0062_8463, NOP, NOP, NOP], &sites).unwrap();
    for (word, diagnostic) in [
        (0x0062_9463, "funct3: 1, rs1: 5, rs2: 6"),
        (0x0063_0463, "funct3: 0, rs1: 6, rs2: 6"),
        (0x0052_8463, "funct3: 0, rs1: 5, rs2: 5"),
        (0x0062_8663, "targets 0x8000000c, expected 0x80000008"),
    ] {
        let error = check_words(&[word, NOP, NOP, NOP], &sites).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn rejects_assembler_accepted_halfword_target() {
    let bytes = assemble(
        "jal zero, 6\naddi zero, zero, 0\naddi zero, zero, 0",
        Arch::Rv64,
    )
    .unwrap();
    let image = Image {
        bytes: &bytes,
        base_pc: ENTRY_PC,
    };
    assert_eq!(image.word(ENTRY_PC), Ok(0x0060_006f));
    let error = image
        .verify_block(ENTRY_PC, 3, &[Site::jal(0, 0, ENTRY_PC + 6)])
        .unwrap_err();
    assert!(
        error.contains("PC 0x80000006 is not four-byte aligned"),
        "{error}"
    );
}

#[test]
fn checks_full_word_site_and_target_bounds() {
    let words = [0x0080_006f, NOP, NOP].map(u32::to_le_bytes);
    let bytes = words.as_flattened();
    let sites = [Site::jal(0, 0, ENTRY_PC + 8)];
    for (len, start, diagnostic) in [
        (3, ENTRY_PC, "full word at PC 0x80000000 is outside"),
        (8, ENTRY_PC, "full word at PC 0x80000008 is outside"),
        (11, ENTRY_PC, "full word at PC 0x80000008 is outside"),
        (12, ENTRY_PC + 2, "PC 0x80000002 is not four-byte aligned"),
        (12, ENTRY_PC + 1, "PC 0x80000001 is not four-byte aligned"),
        (12, ENTRY_PC - 4, "full word at PC 0x7ffffffc is outside"),
    ] {
        let image = Image {
            bytes: &bytes[..len],
            base_pc: ENTRY_PC,
        };
        let error = image.verify_block(start, 1, &sites).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
    check_words(&[0x0080_006f, NOP, NOP], &sites).unwrap();
    let error = check_words(&[0xffdf_f06f], &[Site::jal(0, 0, ENTRY_PC - 4)]).unwrap_err();
    assert!(
        error.contains("full word at PC 0x7ffffffc is outside"),
        "{error}"
    );
}

#[test]
fn rejects_pc_arithmetic_wraparound() {
    for (site, word) in [(0, 0xffdf_f06f_u32), (u64::MAX - 3, 0x0080_006f)] {
        let image = Image {
            bytes: &word.to_le_bytes(),
            base_pc: site,
        };
        let error = image
            .verify_block(site, 1, &[Site::jal(0, 0, site)])
            .unwrap_err();
        assert!(error.contains("target address overflow"), "{error}");
    }
    assert!(pc_at_word(u64::MAX - 3, 1).is_err());
    assert!(pc_at_word(0, u64::MAX).is_err());
}

#[test]
fn rejects_missing_duplicate_and_inconsistent_site_declarations() {
    let words = [0x0080_006f, NOP, NOP];
    let continuation = ENTRY_PC + 8;
    let missing = check_words(&words, &[]).unwrap_err();
    assert!(missing.contains("unexpected control transfer"), "{missing}");
    for sites in [
        vec![Site::jal(0, 0, continuation), Site::jal(0, 0, continuation)],
        vec![Site::jal(0, 0, continuation), Site::jal(3, 0, continuation)],
        vec![Site::jal(2, 0, continuation), Site::jal(0, 0, continuation)],
    ] {
        let error = check_words(&words, &sites).unwrap_err();
        assert!(
            error.contains("invalid or duplicated site declaration"),
            "{error}"
        );
    }
}

#[test]
fn rejects_unexpected_assembler_branch_expansion() {
    let padding = "addi zero, zero, 0\n".repeat(1249);
    let source = format!("beq t0, t1, destination\n{padding}destination:\naddi zero, zero, 0\n");
    let bytes = assemble(&source, Arch::Rv64).unwrap();
    let image = Image {
        bytes: &bytes,
        base_pc: ENTRY_PC,
    };
    assert_eq!(image.word(ENTRY_PC), Ok(0x0062_9463)); // BNE +8
    assert_eq!(image.word(ENTRY_PC + 4), Ok(0x3880_106f)); // JAL +5000
    let error = image
        .verify_block(ENTRY_PC, 3, &[Site::branch(0, 0, 5, 6, ENTRY_PC + 5080)])
        .unwrap_err();
    assert!(error.contains("funct3: 1"), "{error}");
    let error = image
        .verify_block(ENTRY_PC, 3, &[Site::branch(0, 1, 5, 6, ENTRY_PC + 8)])
        .unwrap_err();
    assert!(
        error.contains("unexpected control transfer 0x3880106f at 0x80000004"),
        "{error}"
    );
}

#[test]
fn rejects_undeclared_indirect_transfers_and_non_base_widths() {
    for (word, diagnostic) in [
        (0x0000_8067, "unexpected control transfer"),
        (0x0001_0001, "unsupported instruction width"),
        (0x0000_001f, "unsupported instruction width"),
    ] {
        let error = check_words(&[word], &[]).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn rejects_wrong_roles_and_local_anchors_in_reassembled_mixed_image() {
    let compiled = compile_evm_bytecode_legacy(MIXED_INPUT).unwrap();
    let publishers = compiled.termination_entry_pcs();
    assert_eq!(publishers, [ENTRY_PC + 472, ENTRY_PC + 508, ENTRY_PC + 548]);
    for (original, mutated, diagnostic) in [
        (
            "jal zero, 8\n    jal zero, 468",
            "jal zero, 12\n    jal zero, 468",
            "targets 0x80000030, expected 0x8000002c",
        ),
        (
            "guest_dup1_underflow_1:\n    jal zero, 448",
            "guest_dup1_underflow_1:\n    jal zero, 408",
            "targets 0x800001fc, expected 0x80000224",
        ),
        (
            "guest_dup1_overflow_1:\n    jal zero, 404",
            "guest_dup1_overflow_1:\n    jal zero, 444",
            "targets 0x80000224, expected 0x800001fc",
        ),
        (
            "jal ra, evm_add_3",
            "jal ra, 20",
            "targets 0x800000a4, expected 0x800000a0",
        ),
        (
            "jal ra, evm_sub_4",
            "jal zero, evm_sub_4",
            "Jal { rd: 0 }, expected Jal { rd: 1 }",
        ),
        (
            "    ret\n",
            "    jalr zero, 4(ra)\n",
            "word 0x00408067, expected 0x00008067",
        ),
        (
            "jal ra, evm_add_3\n    addi s1, s1, -32",
            "jal ra, evm_add_3\n    addi s1, s1, -64",
            "word 0xfc048493, expected 0xfe048493",
        ),
        (
            "evm_stop:\n    jal zero, 4",
            "evm_stop:\n    jal zero, 40",
            "targets 0x800001fc, expected 0x800001d8",
        ),
    ] {
        let source = compiled.assembly().replacen(original, mutated, 1);
        assert_ne!(source, compiled.assembly(), "missing fixture {original:?}");
        let bytes = assemble(&source, Arch::Rv64).unwrap();
        assert_ne!(bytes, compiled.machine_code());
        let error = verify_final_image(&bytes, MIXED_FRAGMENTS, PublisherLayout::Mixed, publishers)
            .unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn verifies_fragment_and_publisher_inventory_and_publication_roles() {
    let compiled = compile_evm_bytecode_legacy(MIXED_INPUT).unwrap();
    let code = compiled.machine_code();
    let publishers = compiled.termination_entry_pcs();
    let verify = |bytes: &[u8], fragments: &[Fragment], pcs: &[u64]| {
        verify_final_image(
            bytes,
            fragments.iter().copied(),
            PublisherLayout::Mixed,
            pcs,
        )
    };
    verify(code, &MIXED_FRAGMENTS, publishers).unwrap();
    assert!(verify(code, &MIXED_FRAGMENTS[..5], publishers).is_err());
    let mut duplicate = MIXED_FRAGMENTS.to_vec();
    duplicate.insert(3, Fragment::Pop);
    assert!(verify(code, &duplicate, publishers).is_err());
    assert!(verify(code, &MIXED_FRAGMENTS, &publishers[..2]).is_err());
    for wrong in [
        [publishers[0], publishers[1], publishers[1]],
        [publishers[0], publishers[2], publishers[1]],
        [publishers[0] + 2, publishers[1], publishers[2]],
    ] {
        assert!(verify(code, &MIXED_FRAGMENTS, &wrong).is_err());
    }
    // Both exceptional blocks have the same width. Swapping the two keeps
    // every declared transfer unchanged but must fail the role check.
    let mut wrong = code.to_vec();
    wrong[508..588].rotate_left(40);
    let error = verify(&wrong, &MIXED_FRAGMENTS, publishers).unwrap_err();
    assert!(error.contains("Overflow publisher"), "{error}");
    wrong.copy_from_slice(code);
    wrong[472 + 12..472 + 24].rotate_left(8);
    let error = verify(&wrong, &MIXED_FRAGMENTS, publishers).unwrap_err();
    assert!(error.contains("Normal publisher"), "{error}");
    wrong.copy_from_slice(code);
    wrong.extend(NOP.to_le_bytes());
    let error = verify(&wrong, &MIXED_FRAGMENTS, publishers).unwrap_err();
    assert!(
        error.contains("unexpected image bytes after publisher end"),
        "{error}"
    );
}

#[test]
fn declared_publisher_at_zero_cannot_hide_a_missing_body() {
    let program = compile_evm_bytecode(&[]).unwrap();
    let normal_only = &program.machine_code()[..36];
    for (layout, pcs) in [
        (PublisherLayout::Symbolic, vec![0]),
        (
            PublisherLayout::SymbolicNormalOverflowOutOfGas,
            vec![ENTRY_PC, 0, 0],
        ),
    ] {
        let bytes = if pcs.len() == 1 { &[][..] } else { normal_only };
        assert!(verify_final_image(bytes, [], layout, &pcs).is_err());
    }
}

#[test]
fn rejects_missing_gas_stores_and_premature_status() {
    let compiled = compile_evm_bytecode(&[0x80]).unwrap();
    let pcs = compiled.termination_entry_pcs();
    for (index, &entry) in pcs.iter().enumerate() {
        let start = (entry - ENTRY_PC) as usize;
        let (stores, status): (&[usize], usize) = if index == 0 {
            (&[20], 28)
        } else {
            (&[20, 24], 32)
        };
        let rejects = |bytes: &[u8]| {
            let error = verify_final_image(
                bytes,
                [Fragment::MeteredDup1],
                PublisherLayout::SymbolicNormalUnderflowOverflowOutOfGas,
                pcs,
            )
            .unwrap_err();
            assert!(error.contains("publisher at"), "{error}");
        };
        for &store in stores {
            let mut bytes = compiled.machine_code().to_vec();
            bytes[start + store..start + store + 4].copy_from_slice(&NOP.to_le_bytes());
            rejects(&bytes);
        }
        // Publish the correct terminal status before either gas instruction.
        let gas_start = status - 12;
        let mut bytes = compiled.machine_code().to_vec();
        bytes[start + gas_start..start + status + 4].rotate_left(8);
        rejects(&bytes);
    }
}

#[test]
fn metered_stack_and_arithmetic_reject_corrupted_charge_words_and_targets() {
    for (opcode, fragment, layout, cost) in [
        (
            0x50,
            Fragment::MeteredPop,
            PublisherLayout::SymbolicNormalUnderflowOutOfGas,
            2,
        ),
        (
            0x80,
            Fragment::MeteredDup1,
            PublisherLayout::SymbolicNormalUnderflowOverflowOutOfGas,
            3,
        ),
        (
            0x01,
            Fragment::MeteredArithmetic(ArithmeticCore::Add),
            PublisherLayout::SymbolicNormalUnderflowOutOfGas,
            3,
        ),
        (
            0x03,
            Fragment::MeteredArithmetic(ArithmeticCore::Sub),
            PublisherLayout::SymbolicNormalUnderflowOutOfGas,
            3,
        ),
    ] {
        let program = compile_evm_bytecode(&[opcode]).unwrap();
        let pcs = program.termination_entry_pcs();
        let verify = |bytes: &[u8]| verify_final_image(bytes, [fragment], layout, pcs);
        verify(program.machine_code()).unwrap();
        for (from, to, diagnostic) in [
            (
                format!("addi t2, zero, {cost}"),
                "addi t2, zero, 1",
                "expected 0x00",
            ),
            ("bgeu t0, t2".to_owned(), "bge t0, t2", "funct3: 7"),
            (
                "sub t0, t0, t2".to_owned(),
                "addi zero, zero, 0",
                "expected 0x407282b3",
            ),
            (
                "sub t0, t0, t2".to_owned(),
                "add t0, t0, t2",
                "expected 0x407282b3",
            ),
            (
                "sd t0, 32(t1)".to_owned(),
                "sd t0, 40(t1)",
                "expected 0x02533023",
            ),
            (
                "jal zero, out_of_gas_exit".to_owned(),
                "jal zero, binary_stack_underflow_exit",
                "targets",
            ),
            (
                "jal zero, normal_exit".to_owned(),
                "jal zero, binary_stack_underflow_exit",
                "targets",
            ),
        ] {
            let mutant = program.assembly().replacen(&from, to, 1);
            assert_ne!(mutant, program.assembly(), "opcode {opcode:#04x}, {from}");
            let bytes = assemble(&mutant, Arch::Rv64).unwrap();
            let error = verify(&bytes).unwrap_err();
            assert!(
                error.contains(diagnostic),
                "opcode {opcode:#04x}, {from}: {error}"
            );
        }
    }
}

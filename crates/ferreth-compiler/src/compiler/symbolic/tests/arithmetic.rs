use super::{
    pop_dup1::{observe_guest, FaultPath},
    *,
};
use crate::{execution::Rv64Runner, Opcode};

const ADD: u8 = 1;
const SUB: u8 = 3;
const POP: u8 = 0x50;
const DUP1: u8 = 0x80;
const ARITH_UNDERFLOW: &[(u64, u32)] = &[(0, 0x13), (4, 0x63), (32, 0x6f)];
const POP_UNDERFLOW: &[(u64, u32)] = &[(0, 0x63), (12, 0x6f)];
const DUP_UNDERFLOW: &[(u64, u32)] = &[(0, 0x63), (56, 0x6f)];
const DUP_OVERFLOW: &[(u64, u32)] = &[(0, 0x63), (4, 0x37), (8, 0x33), (12, 0x63), (60, 0x6f)];
const PUSH_OVERFLOW: &[(u64, u32)] = &[(0, 0x37), (4, 0x33), (8, 0x63), (40, 0x6f)];

fn standalone_core(opcode: Opcode) -> Vec<u8> {
    let source = opcode.assembly();
    let instructions = source
        .lines()
        .filter(|line| !line.trim_start().starts_with('.'))
        .collect::<Vec<_>>()
        .join("\n");
    asm_rs::assemble(&instructions, Arch::Rv64).unwrap()
}

fn assert_artifacts(input: &[u8], candidate: &CompiledRv64Program, legacy: &CompiledRv64Program) {
    let decoded = decode_detailed(input).unwrap();
    let mut body_size = 0;
    let mut relocations = Vec::new();
    let mut labels = 2; // Normal and Underflow.
    let mut overflow = false;
    let image = assemble_image(candidate.assembly()).unwrap();
    assert_eq!(image.bytes(), candidate.machine_code());
    assert_eq!(
        assemble_image(legacy.assembly()).unwrap().bytes(),
        legacy.machine_code()
    );
    for (index, instruction) in decoded.iter().enumerate() {
        body_size += match instruction {
            DecodedInstruction::Stop { .. } => {
                labels += 1;
                4
            }
            DecodedInstruction::Push { opcode: 0x60, .. } => {
                labels += 3;
                overflow = true;
                relocations.push(body_size + 40);
                44
            }
            DecodedInstruction::Pop { .. } => {
                labels += 2;
                relocations.push(body_size + 12);
                16
            }
            DecodedInstruction::Dup1 { .. } => {
                labels += 3;
                overflow = true;
                relocations.extend([body_size + 56, body_size + 60]);
                64
            }
            DecodedInstruction::Add { .. } | DecodedInstruction::Sub { .. } => {
                labels += 4;
                let opcode = if matches!(instruction, DecodedInstruction::Add { .. }) {
                    Opcode::Add
                } else {
                    Opcode::Sub
                };
                let core = standalone_core(opcode);
                assert_eq!(core.len(), 136);
                assert_eq!(
                    candidate.machine_code()[body_size + 36..body_size + 172],
                    core
                );
                assert_eq!(legacy.machine_code()[body_size + 36..body_size + 172], core);
                for (name, offset) in [
                    ("guest_arithmetic", 0),
                    ("arith_update", 24),
                    ("arith_underflow", 32),
                    ("arith_core", 36),
                ] {
                    assert_eq!(
                        image.label_address(&format!("{name}_{index}")),
                        Some(ENTRY_PC + (body_size + offset) as u64)
                    );
                }
                relocations.push(body_size + 32);
                172
            }
            _ => unreachable!(),
        };
    }
    assert_eq!(image.labels().len(), labels + usize::from(overflow));
    let normal = ENTRY_PC + body_size as u64;
    let mut candidate_pcs = vec![normal, normal + 36];
    if overflow {
        candidate_pcs.push(normal + 76);
    }
    assert_eq!(candidate.termination_entry_pcs(), candidate_pcs);
    assert_eq!(
        legacy.termination_entry_pcs(),
        [normal, normal + 36, normal + 76]
    );
    assert_eq!(
        candidate.machine_code().len(),
        body_size + 76 + 40 * usize::from(overflow)
    );
    assert_eq!(legacy.machine_code().len(), body_size + 116);
    assert_eq!(candidate.normal_termination_instructions(), 9);
    assert_eq!(legacy.normal_termination_instructions(), 9);
    assert_eq!(
        candidate.exceptional_termination_instructions(),
        10 + 10 * u64::from(overflow)
    );
    assert_eq!(legacy.exceptional_termination_instructions(), 10);
    let c = candidate.machine_code();
    let l = legacy.machine_code();
    let changed: Vec<_> = (0..body_size)
        .step_by(4)
        .filter(|&at| c[at..at + 4] != l[at..at + 4])
        .collect();
    assert_eq!(changed, relocations);
    assert_eq!(c[body_size..body_size + 36], l[body_size..body_size + 36]);
    assert_eq!(
        c[body_size + 36..body_size + 76],
        l[body_size + 76..body_size + 116]
    );
    if overflow {
        assert_eq!(c[body_size + 76..], l[body_size + 36..body_size + 76]);
    }
    println!("input={input:02x?} image_bytes={}/{} publishers={:x?}/{:x?} relocated_body_offsets={changed:x?}",
        c.len(), l.len(), candidate_pcs, legacy.termination_entry_pcs());
}

fn parity(
    input: &[u8],
    initial: &[U256],
    expected: &[U256],
    body_words: u64,
    calls: &[(u64, usize)],
    failure: Option<(u64, FaultPath)>,
) {
    let candidate = compile(input).unwrap();
    let legacy = crate::compiler::compile_evm_bytecode_legacy(input).unwrap();
    assert_artifacts(input, &candidate, &legacy);
    let normal = candidate.termination_entry_pcs()[0];
    let (code, fault) = failure.map_or((0, None), |(code, path)| (code, Some(path)));
    let offsets = [(0, 0), (76, 36), (36, 76)];
    let (c_offset, l_offset) = offsets[code as usize];
    let actual = observe_guest(&candidate, initial, normal + c_offset, code, fault, calls);
    let reference = observe_guest(&legacy, initial, normal + l_offset, code, fault, calls);
    assert_guest_result(&actual, expected, code, body_words);
    assert_guest_result(&reference, expected, code, body_words);
    assert_eq!(actual.guest, reference.guest);
    assert_eq!(actual.trace, reference.trace);
}

#[test]
fn standalone_arithmetic_core_words_and_fingerprints_match() {
    for (bytecode, opcode, hash) in [
        (ADD, Opcode::Add, 0xf1ff_f32f_f479_62f3),
        (SUB, Opcode::Sub, 0x9f00_8f32_0eeb_7a7b),
    ] {
        let core = standalone_core(opcode);
        let candidate = compile(&[bytecode, bytecode]).unwrap();
        assert_eq!(core.len(), 34 * 4);
        for offset in [0, 172] {
            assert_eq!(candidate.machine_code()[offset + 36..offset + 172], core);
        }
        let actual = core.iter().fold(0xcbf2_9ce4_8422_2325_u64, |acc, &byte| {
            (acc ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        });
        assert_eq!(actual, hash);
        let runner = Rv64Runner::new(opcode).unwrap();
        assert_eq!(runner.routine_instruction_words(), 34);
        assert_eq!(runner.machine_code_hash(), hash);
    }
}

#[test]
fn replays_all_phase_one_mixed_specimens() {
    let two = U256::from(2);
    let wide = U256::from_limbs([0x1234, 0x5678, u64::MAX, 42]);
    // The 17 Phase 1.6 specimens, including both endings of the repeated program.
    parity(
        &[0x60, 2, 0x60, 3, ADD, 0],
        &[],
        &[U256::from(5)],
        63,
        &[(88, 2)],
        None,
    );
    parity(
        &[0x60, 2, 0x60, 3, SUB, 0],
        &[],
        &[U256::from(1)],
        63,
        &[(88, 2)],
        None,
    );
    parity(&[0x60, 2, POP, 0], &[], &[], 14, &[], None);
    parity(&[0x60, 2, DUP1, 0], &[], &[two, two], 25, &[], None);
    parity(&[DUP1, POP, DUP1], &[wide], &[wide, wide], 31, &[], None);
    for (opcode, words, path) in [
        (ADD, 3, ARITH_UNDERFLOW),
        (POP, 2, POP_UNDERFLOW),
        (DUP1, 2, DUP_UNDERFLOW),
    ] {
        parity(&[opcode, 0], &[], &[], words, &[], Some((2, (0, path))));
    }
    parity(
        &[0x60, 2, SUB, 0],
        &[],
        &[two],
        13,
        &[],
        Some((2, (44, ARITH_UNDERFLOW))),
    );
    parity(
        &[0x60, 2, ADD],
        &[],
        &[two],
        13,
        &[],
        Some((2, (44, ARITH_UNDERFLOW))),
    );

    let full = full_stack();
    parity(
        &[DUP1, 0],
        &full,
        &full,
        5,
        &[],
        Some((1, (0, DUP_OVERFLOW))),
    );
    parity(
        &[0x60, 2, DUP1, 0],
        &full,
        &full,
        4,
        &[],
        Some((1, (0, PUSH_OVERFLOW))),
    );
    let short = &full[..full.len() - 1];
    let mut filled = short.to_vec();
    filled.push(*short.last().unwrap());
    parity(
        &[DUP1, DUP1, 0],
        short,
        &filled,
        19,
        &[],
        Some((1, (64, DUP_OVERFLOW))),
    );
    let high = U256::from(1) << 192;
    parity(
        &[ADD],
        &[U256::MAX, U256::from(1)],
        &[U256::ZERO],
        42,
        &[(0, 2)],
        None,
    );
    parity(
        &[SUB],
        &[U256::from(1), high],
        &[high - U256::from(1)],
        42,
        &[(0, 2)],
        None,
    );

    let mut repeated = vec![
        0x60, 2, 0x60, 3, ADD, DUP1, 0x60, 7, SUB, POP, 0x60, 9, ADD, DUP1, POP, 0x60, 8, SUB,
    ];
    let calls = &[(88, 2), (368, 3), (600, 2), (896, 2)];
    let expected = [U256::MAX - U256::from(5)];
    parity(&repeated, &[], &expected, 252, calls, None);
    repeated.push(0);
    parity(&repeated, &[], &expected, 253, calls, None);
    let stopped = compile(&repeated).unwrap();
    repeated.extend([ADD, SUB, 0xfe, 0x60]);
    assert_eq!(compile(&repeated).unwrap(), stopped);
    parity(&repeated, &[], &expected, 253, calls, None);
}

#[test]
fn arithmetic_boundary_and_repeated_call_vectors() {
    let one = U256::from(1);
    let high = one << 192;
    let wide_a = U256::from_limbs([u64::MAX, 0xfe, 0xabcd, 0xffff_ffff_1234_5678]);
    let wide_b = U256::from_limbs([0xabc, u64::MAX, 0x9876, 0xabcd_0000_1234_5678]);
    for (opcode, a, b, expected) in [
        (ADD, U256::from(2), U256::from(3), U256::from(5)),
        (SUB, U256::from(2), U256::from(3), one),
        (SUB, U256::from(3), U256::from(2), U256::MAX),
        (ADD, high - one, one, high),
        (ADD, wide_a, wide_b, wide_b.wrapping_add(wide_a)),
        (SUB, wide_a, wide_b, wide_b.wrapping_sub(wide_a)),
    ] {
        parity(&[opcode], &[a, b], &[expected], 42, &[(0, 2)], None);
    }
    let full = full_stack();
    for opcode in [ADD, SUB] {
        for initial in [&[][..], &[wide_a][..]] {
            parity(
                &[opcode],
                initial,
                initial,
                3,
                &[],
                Some((2, (0, ARITH_UNDERFLOW))),
            );
        }
        let mut expected = full.clone();
        let top = expected.pop().unwrap();
        let next = expected.last_mut().unwrap();
        *next = if opcode == ADD {
            top.wrapping_add(*next)
        } else {
            top.wrapping_sub(*next)
        };
        parity(&[opcode], &full, &expected, 42, &[(0, 1024)], None);
    }
    for (input, expected) in [([ADD, ADD], 11), ([SUB, SUB], 1), ([ADD, SUB], 7)] {
        parity(
            &input,
            &[2, 3, 6].map(U256::from),
            &[U256::from(expected)],
            84,
            &[(0, 3), (172, 2)],
            None,
        );
    }
    parity(
        &[ADD, SUB],
        &[wide_a, wide_b],
        &[wide_b.wrapping_add(wide_a)],
        45,
        &[(0, 2)],
        Some((2, (172, ARITH_UNDERFLOW))),
    );
    parity(
        &[0x60, 2, 0x60, 3, ADD],
        &[],
        &[U256::from(5)],
        62,
        &[(88, 2)],
        None,
    );
    for (opcode, expected) in [(ADD, 4), (SUB, 0)] {
        parity(
            &[0x60, 2, DUP1, opcode],
            &[],
            &[U256::from(expected)],
            66,
            &[(108, 2)],
            None,
        );
    }
}

fn negative_fragments() -> [SymbolicFragment; 3] {
    [
        generate_symbolic_add(0, "guest_arithmetic_1".to_owned()),
        generate_symbolic_sub(1, "guest_push1_2".to_owned()),
        generate_symbolic_push1(2, 2),
    ]
}

fn assert_rejected(
    fragments: &[SymbolicFragment],
    publishers: &[PublisherRole],
    source: String,
    diagnostic: &str,
) {
    let image = assemble_image(&source).expect("mutation must reach final-image verification");
    let error = verified_program(fragments, publishers, source, image).unwrap_err();
    assert!(
        matches!(&error, CompileError::Link { description } if description.contains(diagnostic)),
        "expected {diagnostic:?}: {error:?}"
    );
}

#[test]
fn arithmetic_verifier_rejects_wrong_machine_words_and_destinations() {
    let fragments = negative_fragments();
    let publishers = publisher_inventory(&fragments);
    let source = emit(&fragments, &publishers);
    for (from, to, diagnostic) in [
        ("jal ra, arith_core_0", "jal ra, arith_core_1", "targets"),
        (
            "jal ra, arith_core_0",
            "jal t0, arith_core_0",
            "Jal { rd: 1 }",
        ),
        (
            "jal ra, arith_core_0",
            ".long 0x012000ef",
            "not four-byte aligned",
        ),
        ("addi s1, s1, -32", "addi s1, s1, -64", "leaf return anchor"),
        (
            "jal zero, guest_arithmetic_1",
            "jal zero, arith_core_1",
            "targets",
        ),
        (
            "jal zero, binary_stack_underflow_exit",
            "jal zero, normal_exit",
            "targets",
        ),
        (
            "jal zero, binary_stack_underflow_exit",
            "jal zero, push1_stack_overflow_exit",
            "targets",
        ),
        ("li t3, 0", "li t3, 1", "arithmetic at"),
        (
            "li t3, 0",
            "jal zero, arith_update_0",
            "unexpected control transfer",
        ),
        ("ret", "jalr zero, 0(t0)", "expected leaf return"),
        ("ret", "jalr zero, 4(ra)", "expected leaf return"),
        ("addi a0, s1, -32", "addi a0, s1, -64", "arithmetic at"),
        (
            "bltu s1, t0, arith_underflow_0",
            "bgeu s1, t0, 8\n    jal zero, arith_underflow_0",
            "expected",
        ),
    ] {
        assert!(source.contains(from), "missing mutation site {from}");
        assert_rejected(
            &fragments,
            &publishers,
            source.replacen(from, to, 1),
            diagnostic,
        );
    }
}

#[test]
fn arithmetic_verifier_rejects_incorrect_or_incomplete_declarations() {
    let originals = negative_fragments();
    let publishers = publisher_inventory(&originals);
    let source = emit(&originals, &publishers);
    let mut fragments = originals.clone();
    let call = fragments[0].local_transfers.remove(1);
    assert_rejected(
        &fragments,
        &publishers,
        source.clone(),
        "unexpected control transfer",
    );

    let mut fragments = originals.clone();
    fragments[0].local_transfers.push(call);
    assert_rejected(
        &fragments,
        &publishers,
        source.clone(),
        "duplicate symbolic transfer",
    );

    let mut fragments = originals.clone();
    fragments[0].local_transfers[2].target_symbol = "arith_update_1".into();
    assert_rejected(
        &fragments,
        &publishers,
        source.clone(),
        "leaf return anchor",
    );

    // The declaration scan accepts these mutually consistent bytes and
    // declarations. The independent layout still requires the missing call.
    let mut fragments = originals.clone();
    fragments[0]
        .local_transfers
        .retain(|transfer| !matches!(transfer.site_word, 5 | 42));
    let edited = source
        .replacen("jal ra, arith_core_0", "nop", 1)
        .replacen("ret", "nop", 1);
    assert_rejected(
        &fragments,
        &publishers,
        edited,
        "at declared transfer 0x80000014",
    );

    // Swapping both calls and their return declarations produces internally
    // consistent bindings to another occurrence, which the fixed layout rejects.
    let mut fragments = originals.clone();
    fragments[0].local_transfers[1].target_symbol = "arith_core_1".into();
    fragments[1].local_transfers[1].target_symbol = "arith_core_0".into();
    fragments[0].local_transfers[2].target_symbol = "arith_update_1".into();
    fragments[1].local_transfers[2].target_symbol = "arith_update_0".into();
    let edited = source
        .replacen("jal ra, arith_core_0", "jal ra, arith_core_1", 1)
        .replacen(
            "jal ra, arith_core_1\narith_update_1:",
            "jal ra, arith_core_0\narith_update_1:",
            1,
        );
    assert_rejected(
        &fragments,
        &publishers,
        edited,
        "transfer at 0x80000014 targets",
    );

    let mut fragments = originals;
    fragments[0].external_references[0].role = PublisherRole::Overflow;
    assert_rejected(
        &fragments,
        &publishers,
        source.replacen(
            "jal zero, binary_stack_underflow_exit",
            "jal zero, push1_stack_overflow_exit",
            1,
        ),
        "transfer at 0x80000020 targets",
    );
}

#[test]
fn repeated_arithmetic_rejects_colliding_entry_and_leaf_names() {
    let normal = PublisherRole::Normal.label().to_owned();
    for repeated in [generate_symbolic_add, generate_symbolic_sub] {
        let error = compose(&[
            generate_symbolic_add(0, normal.clone()),
            repeated(0, normal.clone()),
        ])
        .unwrap_err();
        assert!(
            matches!(error, CompileError::Link { description } if description.contains("duplicate fragment entry"))
        );
    }

    let mut fragments = negative_fragments();
    fragments[1].source = fragments[1].source.replace("arith_core_1", "arith_core_0");
    assert!(matches!(
        compose(&fragments),
        Err(CompileError::Link { .. })
    ));
}

#[test]
fn replays_cancun_multilimb_dup_and_stopped_sub_vectors() {
    // Same inputs as directed_rc_programs_match_revm_cancun.
    parity(
        &[0x60, 1, 0x60, 0, SUB, DUP1],
        &[],
        &[U256::MAX, U256::MAX],
        76,
        &[(88, 2)],
        None,
    );
    parity(
        &[0x60, 5, DUP1, SUB, 0],
        &[],
        &[U256::ZERO],
        67,
        &[(108, 2)],
        None,
    );
}

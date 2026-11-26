use super::*;
use crate::{
    orchestrator::execute_guest_evm_bytecode,
    rv64::{guest::GUEST_METADATA_LIVE_REMAINING_OFFSET, memory::EXIT_PC},
};

#[test]
fn gas_writes_each_limb_before_advancing_the_stack_pointer() {
    let program = compile(&[0x5a]).unwrap();
    let mut executor = RvemuExecutor::new().unwrap();
    executor.load_code(program.machine_code()).unwrap();
    let limit = u64::MAX;
    initialize_guest_state(&mut executor, limit).unwrap();
    let base = EVM_STACK_REGION.start();
    let slot = base + EVM_WORD_SIZE;
    let s1 = IntegerRegister::EVM_STACK_NEXT_FREE;
    executor.write_register(s1, slot);

    // Surround the destination with distinct live, dirty, and guarded bytes.
    let mut expected = [0xa5; 128];
    expected[32..64].fill(0xff);
    executor.write_memory(base - 32, &expected).unwrap();
    let funded_path = (0..19).filter(|&word| word != 5 && word != 18);
    let mut steps = funded_path.chain([19]);
    assert_eq!(steps.next(), Some(0));
    let mut word = 0;
    for next in steps {
        executor
            .execute(ENTRY_PC + word * 4, ENTRY_PC + next * 4, 1)
            .unwrap();
        if (12..=15).contains(&word) {
            let offset = 64 + (word as usize - 12) * 8;
            let limb = if word == 12 { limit - 2 } else { 0 };
            expected[offset..offset + 8].copy_from_slice(&limb.to_le_bytes());
        }
        let mut actual = [0; 128];
        executor.read_memory(base - 32, &mut actual).unwrap();
        assert_eq!(actual, expected, "after word {word}");
        assert_eq!(
            executor.read_register(s1),
            slot + if word >= 16 { EVM_WORD_SIZE } else { 0 },
            "after word {word}"
        );
        let mut live = [0; 8];
        executor
            .read_memory(
                METADATA_REGION.start() + GUEST_METADATA_LIVE_REMAINING_OFFSET as u64,
                &mut live,
            )
            .unwrap();
        assert_eq!(
            u64::from_le_bytes(live),
            if word >= 7 { limit - 2 } else { limit }
        );
        word = next;
    }
    assert_eq!(ENTRY_PC + word * 4, program.termination_entry_pcs()[0]);
}

#[test]
fn gas_failures_are_atomic_and_settle_residual_live_gas() {
    use GuestExecutionStatus::{GuestException, Running};

    let full = full_stack();
    for (code, initial, limit, prefix, exception_code, residual, path) in [
        (&[0x5a, 0][..], &[][..], 1, &[][..], 3, 1, OOG_CHARGE_PATH),
        (&[0x5a, 0], &full[..], 1, &full[..], 3, 1, OOG_CHARGE_PATH),
        (&[0x5a, 0], &full[..], 7, &full[..], 1, 5, METERED_OVERFLOW),
        (
            &[0x5a, 0x5a, 0],
            &[][..],
            3,
            &[U256::ONE][..],
            3,
            1,
            OOG_CHARGE_PATH,
        ),
    ] {
        let program = compile(code).unwrap();
        let publisher = program.termination_entry_pcs()[if exception_code == 3 { 2 } else { 1 }];
        let fault_offset = if code.len() == 3 { 76 } else { 0 };
        let observed = pop_dup1::observe_guest(
            &program,
            initial,
            publisher,
            exception_code,
            Some((fault_offset, path)),
            &[],
            limit,
        );
        let pointer = EVM_STACK_REGION.start() + prefix.len() as u64 * EVM_WORD_SIZE;
        assert_eq!(observed.guest.status(), GuestException);
        assert_eq!(observed.guest.exception_code(), exception_code);
        assert_eq!(observed.guest.stack(), prefix);
        assert_eq!(observed.guest.final_s1(), pointer);
        assert_eq!(observed.guest.remaining_gas(), 0);
        assert_eq!(observed.guest.gas_used(), limit);
        let record = |status, code, pointer, live| {
            GuestMetadata::new(status, code, pointer, limit, live, 0).encode()
        };
        assert_eq!(
            observed.trace[0],
            record(Running, 0, EVM_STACK_REGION.start(), residual)
        );
        assert_eq!(
            observed.trace[5],
            record(Running, exception_code, pointer, residual)
        );
        assert_eq!(
            observed.trace[6..9],
            [record(Running, exception_code, pointer, 0); 3]
        );
        assert_eq!(
            observed.trace[9..],
            [record(GuestException, exception_code, pointer, 0); 2]
        );

        let body = observed.body.executed_instructions;
        let termination = observed.termination.executed_instructions;
        assert_eq!(
            body,
            path.len() as u64 + if fault_offset == 0 { 0 } else { 17 }
        );
        assert_eq!(termination, 10);
        println!("GAS bytes={code:02x?} budget={limit} initial_depth={} code={exception_code} live_at_publisher={residual} image_bytes={} RV64={body}+{termination}={}", initial.len(), program.machine_code().len(), body + termination);
    }
}

#[test]
fn gas_layout_and_success_instruction_counts() {
    for (code, stop_bytes, body, size) in [(&[0x5a][..], 0, 17, 192), (&[0x5a, 0][..], 4, 18, 196)]
    {
        let program = compile(code).unwrap();
        let normal = program.termination_entry_pcs()[0];
        let fragment_bytes = normal - ENTRY_PC - stop_bytes;
        assert_eq!(fragment_bytes, 76);
        assert_eq!(fragment_bytes / RV64_INSTRUCTION_SIZE, 19);
        assert_eq!(program.machine_code().len(), size);
        let run = execute_guest_evm_bytecode(code, 10).unwrap();
        assert_eq!(run.guest_result().stack(), [U256::from(8)]);
        assert_eq!(run.guest_result().remaining_gas(), 8);
        let metrics = run.instruction_metrics();
        assert_eq!(metrics.opcode_body(), body);
        assert_eq!(metrics.guest_termination(), 9);
        assert_eq!(metrics.total(), body + 9);
        println!("GAS bytes={code:02x?} budget=10 fragment_bytes={fragment_bytes} image_bytes={size} RV64={body}+9={}", metrics.total());
    }
    assert_eq!(compile(&[0, 0x5a]), compile(&[0]));
}

#[test]
fn gas_reaches_both_publishers_beyond_conditional_branch_range() {
    let program = compile(&[0x5a; 55]).unwrap();
    let full = full_stack();
    for (initial, limit, code, publisher, path, site, displacement) in [
        (
            &[][..],
            1,
            3,
            program.termination_entry_pcs()[2],
            OOG_CHARGE_PATH,
            20,
            4_236,
        ),
        (
            &full[..],
            2,
            1,
            program.termination_entry_pcs()[1],
            METERED_OVERFLOW,
            72,
            4_144,
        ),
    ] {
        assert_eq!(publisher - (ENTRY_PC + site), displacement);
        let observed = pop_dup1::observe_guest(
            &program,
            initial,
            publisher,
            code,
            Some((0, path)),
            &[],
            limit,
        );
        assert_eq!(observed.guest.stack(), initial);
        assert_eq!(observed.guest.exception_code(), code);
        assert_eq!(observed.guest.remaining_gas(), 0);
        assert_eq!(observed.body.executed_instructions, path.len() as u64);
        println!("55 GAS budget={limit} code={code} JAL displacement={displacement}");
    }
}

#[test]
#[ignore = "assembles multi-megabyte GAS sources at the ABI image boundary"]
fn gas_capacity_boundary() {
    // 19 words per GAS, 1 per STOP, 29 across the three publishers.
    // The first OOG JAL is at byte 20 and targets the last 40 bytes.
    let reservation = (EXIT_PC - ENTRY_PC) as usize;
    let aligned_jal_limit = 1_048_572;
    for stop in [false, true] {
        let stop_bytes = 4 * usize::from(stop);
        let size = |n| 76 * n + stop_bytes + 116;
        let last_layout = (reservation - size(0)) / 76;
        let last_relocation = (aligned_jal_limit + 60 - size(0)) / 76;
        assert_eq!((last_layout, last_relocation), (13_795, 13_796));
        let input = |n| {
            vec![0x5a; n]
                .into_iter()
                .chain(stop.then_some(0))
                .collect::<Vec<_>>()
        };

        let program = compile(&input(last_layout)).unwrap();
        assert_eq!(program.machine_code().len(), size(last_layout));
        let image = assemble_image(program.assembly()).unwrap();
        let labels = 3 * last_layout + usize::from(stop) + 3;
        assert_eq!(image.labels().len(), labels);
        let target = program.termination_entry_pcs()[2];
        assert_eq!(target - (ENTRY_PC + 20), (size(last_layout) - 60) as u64);
        for (budget, code, depth, body) in [(0, 3, 0, 6), (2_050, 1, 1_024, 17 * 1_024 + 11)] {
            let run = execute_guest_evm_bytecode(&input(last_layout), budget).unwrap();
            assert_eq!(run.machine_code(), program.machine_code());
            let guest = run.guest_result();
            assert_eq!(guest.exception_code(), code);
            assert_eq!(guest.remaining_gas(), 0);
            assert_eq!(guest.gas_used(), budget);
            assert_eq!(guest.stack().len(), depth);
            assert!(guest
                .stack()
                .iter()
                .enumerate()
                .all(|(i, &word)| word == U256::from(budget - 2 * (i as u64 + 1))));
            let metrics = run.instruction_metrics();
            assert_eq!(metrics.opcode_body(), body);
            assert_eq!(metrics.guest_termination(), 10);
            assert_eq!(metrics.total(), body + 10);
            println!("GAS n={last_layout} STOP={stop} image_bytes={} labels={labels} first_oog_disp={} budget={budget} depth={depth} code={code} RV64={body}+10={}", size(last_layout), size(last_layout) - 60, metrics.total());
        }

        assert_eq!(
            compile(&input(last_layout + 1)),
            Err(CompileError::Layout(LayoutError::CodeOverlapsExitSlot {
                code_end: ENTRY_PC + size(last_layout + 1) as u64,
            }))
        );
        let rejected = last_relocation + 1;
        let error = compile(&input(rejected)).unwrap_err();
        let displacement = size(rejected) - 60;
        assert!(
            error.to_string().contains(&format!(
            "BranchOutOfRange {{ label: \"out_of_gas_exit\", disp: {displacement}, max: 1048574"
        )),
            "{error}"
        );
        println!(
            "GAS n={} STOP={stop} assembles to {} bytes, rejected by reservation",
            last_layout + 1,
            size(last_layout + 1)
        );
        println!("GAS n={rejected} STOP={stop} {error}");
    }
}

#[test]
fn gas_verifier_rejects_corruption_even_when_declarations_agree() {
    use crate::rv64::opcodes::gas::generate_symbolic_gas;

    for (word, original, mutated, diagnostic) in [
        (
            4,
            "bgeu t0, t2",
            "bge t0, t2",
            "expected Branch { funct3: 7",
        ),
        (
            5,
            "jal zero, out_of_gas_exit",
            "jal zero, push1_stack_overflow_exit",
            "targets 0x80000074, expected 0x8000009c",
        ),
        (
            17,
            "jal zero, guest_stop",
            "jal zero, normal_exit",
            "targets 0x80000050, expected 0x8000004c",
        ),
        (
            18,
            "jal zero, push1_stack_overflow_exit",
            "jal zero, out_of_gas_exit",
            "targets 0x8000009c, expected 0x80000074",
        ),
    ] {
        let mut fragments = [
            generate_symbolic_gas(0, "guest_stop".to_owned()),
            stop_fragment(),
        ];
        let publishers = publisher_inventory(&fragments);
        let fragment = &mut fragments[0];
        let source = fragment.source.replacen(original, mutated, 1);
        assert_ne!(source, fragment.source);
        fragment.source = source;
        match word {
            4 => {
                fragment.local_transfers[0].kind = Transfer::Branch {
                    funct3: 5,
                    rs1: 5,
                    rs2: 7,
                }
            }
            5 => fragment.external_references[0].role = PublisherRole::Overflow,
            17 => {
                fragment.local_transfers[2].target_symbol = PublisherRole::Normal.label().to_owned()
            }
            18 => fragment.external_references[1].role = PublisherRole::OutOfGas,
            _ => unreachable!(),
        }
        let source = emit(&fragments, &publishers);
        let image = assemble_image(&source).unwrap();

        let declaration = |site_word: u64, kind, target_symbol: &str| DeclaredTransfer {
            site_pc: ENTRY_PC + site_word * 4,
            target_pc: image.label_address(target_symbol).unwrap(),
            kind,
        };
        let gas = &fragments[0];
        let declarations: Vec<_> = gas
            .local_transfers
            .iter()
            .map(|item| declaration(item.site_word, item.kind, &item.target_symbol))
            .chain(
                gas.external_references
                    .iter()
                    .map(|item| declaration(item.site_word, item.kind, item.role.label())),
            )
            .chain([declaration(
                19,
                Transfer::Jal { rd: 0 },
                PublisherRole::Normal.label(),
            )])
            .collect();
        verify_declared_transfers(image.bytes(), ENTRY_PC + 80, &declarations).unwrap();

        let error = verified_program(&fragments, &publishers, source, image).unwrap_err();
        assert!(
            error.to_string().contains(diagnostic),
            "word {word}: {error}"
        );
    }
}

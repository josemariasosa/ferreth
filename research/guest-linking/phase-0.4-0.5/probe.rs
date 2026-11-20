use asm_rs::{Arch, Assembler, assemble};
use ferreth_compiler::{
    compiler::{CompileError, compile_evm_bytecode},
    orchestrator::execute_guest_evm_bytecode,
    rv64::{
        guest::GuestExecutionStatus,
        memory::{ENTRY_PC, EXIT_PC, validate_generated_code},
        opcodes::push::generate_guest_push1,
    },
};

include!("encoders.rs");

fn read_word(code: &[u8]) -> u32 {
    u32::from_le_bytes(code[..4].try_into().unwrap())
}

fn jal_offset(word: u32) -> i64 {
    assert_eq!(word & 0xfff, 0x06f, "expected JAL x0");
    let immediate = ((word >> 31) << 20)
        | (((word >> 12) & 0xff) << 12)
        | (((word >> 20) & 1) << 11)
        | (((word >> 21) & 0x3ff) << 1);
    i64::from((immediate as i32) << 11 >> 11)
}

fn main() {
    let push = generate_guest_push1(2);
    let site = push.overflow_trampoline_offset();
    let push_size = push.machine_code().len();
    for stop in [false, true] {
        let mut preceding = None;
        for count in [46, 47] {
            let mut bytes = [0x60, 0x02].repeat(count);
            if stop {
                bytes.push(0);
            }
            let result = compile_evm_bytecode(&bytes);
            let (target_offset, code_len) = match (&result, preceding) {
                (Ok(program), None) => {
                    let target = program.termination_entry_pcs()[1] - program.entry_pc();
                    assert_eq!(
                        site as i64 + jal_offset(read_word(&program.machine_code()[site..])),
                        target as i64
                    );
                    let execution = execute_guest_evm_bytecode(&bytes).unwrap();
                    let guest = execution.guest_result();
                    assert_eq!(guest.status(), GuestExecutionStatus::Normal);
                    assert_eq!(guest.stack().len(), count);
                    assert!(guest.stack().iter().all(|value| *value == 2u8));
                    println!(
                        "46 pushes stop={stop}: NORMAL depth={} total={}",
                        guest.stack().len(),
                        execution.instruction_metrics().total()
                    );
                    (target as usize, program.machine_code().len())
                }
                (Err(CompileError::Link { description }), Some((prior_target, prior_len))) => {
                    assert!(description.starts_with("RV64 assembly failed: ImmediateOverflow"));
                    println!("47 pushes stop={stop}: {description}");
                    (prior_target + push_size, prior_len + push_size)
                }
                _ => panic!("unexpected {count} pushes stop={stop}: {result:?}"),
            };
            preceding = Some((target_offset, code_len));
            let displacement = (target_offset - site) as i64;
            assert_eq!(
                displacement,
                match (count, stop) {
                    (46, false) => 2012,
                    (46, true) => 2016,
                    (47, false) => 2056,
                    (47, true) => 2060,
                    _ => unreachable!(),
                }
            );
            let target = ENTRY_PC + target_offset as u64;
            assert!((ENTRY_PC + site as u64).is_multiple_of(4) && target.is_multiple_of(4));
            assert!(target_offset < code_len && ENTRY_PC + code_len as u64 <= EXIT_PC);
            assert!((-1_048_576..=1_048_574).contains(&displacement));
            validate_generated_code(ENTRY_PC, code_len).unwrap();

            let numeric = assemble(&format!("jal zero, {displacement}"), Arch::Rv64);
            match numeric {
                Ok(code) if count == 46 => assert_eq!(jal_offset(read_word(&code)), displacement),
                Err(asm_rs::AsmError::ImmediateOverflow {
                    value, min, max, ..
                }) if count == 47 => {
                    assert_eq!((value, min, max), (i128::from(displacement), -2048, 2047));
                    assert!(
                        format!("{:?}", result.unwrap_err())
                            .contains(&format!("value: {displacement},"))
                    );
                }
                unexpected => panic!("unexpected numeric JAL result: {unexpected:?}"),
            }
            let mut assembler = Assembler::new(Arch::Rv64);
            assembler.base_address(ENTRY_PC + site as u64);
            assembler.define_external("publisher", target);
            assembler.emit("jal zero, publisher").unwrap();
            let relocated = assembler.finish().unwrap();
            let word = read_word(relocated.bytes());
            assert_eq!(jal_offset(word), displacement);
            println!(
                "count={count} stop={stop} site={:#x} target={target:#x} displacement={displacement} code_len={code_len} (47 derived) symbolic_word={word:#010x} relocations={:?}",
                ENTRY_PC + site as u64,
                relocated.relocations()
            );
        }
    }

    for (name, word) in [
        ("compiler", compiler_encoder::encode(-4).unwrap()),
        ("orchestrator", orchestrator_encoder::encode(-4).unwrap()),
    ] {
        let decoded = jal_offset(word);
        assert_eq!((word, decoded), (0x7fdff06f, 1_048_572));
        println!("{name}: requested=-4 word={word:#010x} decoded={decoded:+}");
    }
    let correct = assemble("target:\n    nop\n    jal zero, target", Arch::Rv64).unwrap();
    let word = read_word(&correct[4..]);
    assert_eq!((word, jal_offset(word)), (0xffdff06f, -4));
    println!(
        "assembler: requested=-4 word={word:#010x} decoded={}",
        jal_offset(word)
    );
}

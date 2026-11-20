use asm_rs::{assemble, assemble_at, Arch, AsmError, Assembler, RelocKind};

#[test]
fn rv64_assembler_result() -> Result<(), AsmError> {
    const BASE: u64 = 0x8000_0000;
    const SOURCE: &str = r#"entry:
    addi t0, zero, 42
    jal zero, destination
    addi t0, zero, 99

destination:
    addi t1, zero, 7
"#;

    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.base_address(BASE).emit(SOURCE)?;
    let result = assembler.finish()?;
    let bytes = result.bytes();

    println!("Base address: {:#010x}", result.base_address());
    println!("Final bytes ({}): {:02x?}", result.len(), bytes);
    println!("Labels (name, address): {:x?}", result.labels());
    println!("Applied relocations: {:?}", result.relocations());
    print!("{}", result.listing());

    assert_eq!(result.base_address(), BASE);
    assert_eq!(bytes.len(), 16);
    let (instructions, _) = bytes.as_chunks::<4>();
    let words: Vec<u32> = instructions
        .iter()
        .copied()
        .map(u32::from_le_bytes)
        .collect();
    println!("Instruction words: {words:08x?}");
    assert_eq!(words, [0x02a0_0293, 0x0080_006f, 0x0630_0293, 0x0070_0313]);
    assert_eq!(result.labels().len(), 2);
    assert_eq!(result.label_address("entry"), Some(0x8000_0000));
    assert_eq!(result.label_address("destination"), Some(0x8000_000c));
    assert_eq!(&bytes[4..8], &[0x6f, 0x00, 0x80, 0x00]);

    let [relocation] = result.relocations() else {
        panic!("expected one applied relocation");
    };
    assert_eq!(relocation.offset, 4);
    assert_eq!(relocation.size, 4);
    assert_eq!(relocation.label, "destination");
    assert_eq!(relocation.kind, RelocKind::RvJal20);
    assert_eq!(relocation.addend, 0);
    let jal_pc = result.base_address() + relocation.offset as u64;
    let displacement = result.label_address(&relocation.label).unwrap() as i64 - jal_pc as i64;
    println!("JAL site {jal_pc:#010x}; target minus site = {displacement:+} bytes");
    assert_eq!(jal_pc, 0x8000_0004);
    assert_eq!(displacement, 8);

    let at_base = assemble_at(SOURCE, Arch::Rv64, BASE)?;
    println!("assemble_at bytes: {at_base:02x?}");
    assert_eq!(at_base, bytes);

    let mut zero_base_assembler = Assembler::new(Arch::Rv64);
    zero_base_assembler.emit(SOURCE)?;
    let zero_base = zero_base_assembler.finish()?;
    assert_eq!(zero_base.base_address(), 0);
    let at_zero = assemble(SOURCE, Arch::Rv64)?;
    println!("assemble bytes (base zero): {at_zero:02x?}");
    assert_eq!(at_zero, zero_base.into_bytes());

    Ok(())
}

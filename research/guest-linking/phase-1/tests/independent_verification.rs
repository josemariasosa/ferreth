use asm_rs::{Arch, Assembler, AssemblyResult};

const BASE: u64 = 0x8000_0000;
const NOP: &str = "addi zero, zero, 0\n";

#[derive(Debug, PartialEq, Eq)]
struct Destination {
    displacement: i32,
    pc: u64,
}

impl Destination {
    fn from_immediate(pc: u64, immediate: u32, bits: u32) -> Result<Self, Rejection> {
        let shift = 32 - bits;
        let displacement = ((immediate << shift) as i32) >> shift;
        let pc = pc
            .checked_add_signed(i64::from(displacement))
            .ok_or(Rejection::AddressOverflow)?;
        Ok(Self { displacement, pc })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct BranchFields {
    funct3: u32,
    rs1: u32,
    rs2: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum Rejection {
    Opcode(u32),
    AddressOverflow,
    SiteAlignment(u64),
    SiteBounds(u64),
    TargetAlignment(u64),
    TargetBounds(u64),
    WrongTarget(u64),
    WrongRd(u32),
    WrongBranchFields(BranchFields),
}

fn decode_jal(word: u32, pc: u64) -> Result<(u32, Destination), Rejection> {
    let opcode = word & 0x7f;
    if opcode != 0x6f {
        return Err(Rejection::Opcode(opcode));
    }
    let rd = (word >> 7) & 0x1f;
    let immediate = ((word >> 31) << 20)
        | (((word >> 21) & 0x3ff) << 1)
        | (((word >> 20) & 1) << 11)
        | (word & 0xff000);
    Ok((rd, Destination::from_immediate(pc, immediate, 21)?))
}

fn decode_branch(word: u32, pc: u64) -> Result<(BranchFields, Destination), Rejection> {
    let opcode = word & 0x7f;
    if opcode != 0x63 {
        return Err(Rejection::Opcode(opcode));
    }
    let fields = BranchFields {
        funct3: (word >> 12) & 7,
        rs1: (word >> 15) & 0x1f,
        rs2: (word >> 20) & 0x1f,
    };
    let immediate = ((word >> 31) << 12)
        | (((word >> 25) & 0x3f) << 5)
        | (((word >> 8) & 0xf) << 1)
        | (((word >> 7) & 1) << 11);
    Ok((fields, Destination::from_immediate(pc, immediate, 13)?))
}

fn word_bytes(code: &[u8], pc: u64) -> Option<[u8; 4]> {
    let offset = usize::try_from(pc.checked_sub(BASE)?).ok()?;
    code.get(offset..)?.first_chunk().copied()
}

fn read_word(code: &[u8], pc: u64) -> Result<u32, Rejection> {
    if !pc.is_multiple_of(4) {
        return Err(Rejection::SiteAlignment(pc));
    }
    word_bytes(code, pc)
        .map(u32::from_le_bytes)
        .ok_or(Rejection::SiteBounds(pc))
}

fn verify_target(code: &[u8], actual: u64, expected: u64) -> Result<(), Rejection> {
    if !actual.is_multiple_of(4) {
        return Err(Rejection::TargetAlignment(actual));
    }
    if word_bytes(code, actual).is_none() {
        return Err(Rejection::TargetBounds(actual));
    }
    if actual != expected {
        return Err(Rejection::WrongTarget(actual));
    }
    Ok(())
}

fn verify_jal(code: &[u8], site: u64, expected_rd: u32, expected: u64) -> Result<i32, Rejection> {
    let word = read_word(code, site)?;
    let (rd, actual) = decode_jal(word, site)?;
    if rd != expected_rd {
        return Err(Rejection::WrongRd(rd));
    }
    verify_target(code, actual.pc, expected)?;
    println!("JAL {word:#010x} at {site:#010x}, rd=x{rd}, {actual:?}, expected={expected:#010x}");
    Ok(actual.displacement)
}

fn t0_t1(funct3: u32) -> BranchFields {
    BranchFields {
        funct3,
        rs1: 5,
        rs2: 6,
    }
}

fn verify_branch(code: &[u8], site: u64, funct3: u32, expected: u64) -> Result<i32, Rejection> {
    let word = read_word(code, site)?;
    let (fields, actual) = decode_branch(word, site)?;
    if fields != t0_t1(funct3) {
        return Err(Rejection::WrongBranchFields(fields));
    }
    verify_target(code, actual.pc, expected)?;
    println!(
        "Branch {word:#010x} at {site:#010x}, {fields:?}, {actual:?}, expected={expected:#010x}"
    );
    Ok(actual.displacement)
}

fn assemble(source: &str) -> AssemblyResult {
    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.base_address(BASE).emit(source).unwrap();
    assembler.finish().unwrap()
}

#[test]
fn jal_machine_words() {
    for (word, site, expected_rd, displacement, pc) in [
        (0x0080_006f, 0x8000_0004, 0, 8, 0x8000_000c),
        (0xffdf_f06f, 0x8000_0004, 0, -4, 0x8000_0000),
        (0x0090_006f, 0x8000_0004, 0, 2056, 0x8000_080c),
        (0x8000_006f, 0x8010_0000, 0, -1_048_576, 0x8000_0000),
        (0x7fff_f06f, 0x8000_0000, 0, 1_048_574, 0x800f_fffe),
        (0x0080_00ef, 0x8000_0004, 1, 8, 0x8000_000c),
    ] {
        let (rd, actual) = decode_jal(word, site).unwrap();
        println!("JAL fixture {word:#010x} at {site:#010x}, rd=x{rd}, {actual:?}");
        assert_eq!(rd, expected_rd);
        assert_eq!(actual, Destination { displacement, pc });
    }
}

#[test]
fn branch_machine_words() {
    for (word, funct3, displacement, pc) in [
        (0x0062_8463, 0, 8, 0x8000_0008),
        (0x0062_9463, 1, 8, 0x8000_0008),
        (0xfe62_8ee3, 0, -4, 0x7fff_fffc),
        (0x0062_80e3, 0, 2048, 0x8000_0800),
    ] {
        let (fields, actual) = decode_branch(word, BASE).unwrap();
        println!("Branch fixture {word:#010x} at {BASE:#010x}, {fields:?}, {actual:?}");
        assert_eq!(fields, t0_t1(funct3));
        assert_eq!(actual, Destination { displacement, pc });
    }
    let (rd, actual) = decode_jal(0x3880_106f, BASE + 4).unwrap();
    println!("Relaxed JAL fixture 0x3880106f at 0x80000004, rd=x{rd}, {actual:?}");
    assert_eq!(rd, 0);
    assert_eq!(
        actual,
        Destination {
            displacement: 5000,
            pc: 0x8000_138c,
        }
    );
}

#[test]
fn assembled_jal_destinations() {
    for (source, expected_rd, displacement, expected_pc) in [
        (
            format!("{NOP}jal zero, destination\n.space 2052\ndestination:\n{NOP}"),
            0,
            2056,
            0x8000_080c,
        ),
        (
            format!("destination:\n{NOP}jal zero, destination\n"),
            0,
            -4,
            BASE,
        ),
        (
            format!("{NOP}jal ra, destination\n{NOP}destination:\n{NOP}"),
            1,
            8,
            0x8000_000c,
        ),
    ] {
        let result = assemble(&source);
        let expected = result.label_address("destination").unwrap();
        assert_eq!(expected, expected_pc);
        // Each specimen starts with one explicit ADDI followed by its JAL.
        assert_eq!(
            verify_jal(result.bytes(), BASE + 4, expected_rd, expected),
            Ok(displacement)
        );
    }
}

#[test]
fn assembled_nearby_and_relaxed_branches() {
    for padding_count in [1, 1249] {
        let padding = NOP.repeat(padding_count);
        let source = format!("beq t0, t1, destination\nfallthrough:\n{padding}destination:\n{NOP}");
        let result = assemble(&source);
        let code = result.bytes();
        let fallthrough = result.label_address("fallthrough").unwrap();
        let destination = result.label_address("destination").unwrap();

        if padding_count == 1 {
            assert_eq!(fallthrough, BASE + 4);
            assert_eq!(destination, 0x8000_0008);
            assert_eq!(verify_branch(code, BASE, 0, destination), Ok(8));
        } else {
            assert_eq!(fallthrough, BASE + 8);
            assert_eq!(destination, 0x8000_138c);
            // t0 != t1 takes BNE to the original fallthrough, past the JAL.
            assert_eq!(verify_branch(code, BASE, 1, fallthrough), Ok(8));
            // t0 == t1 falls through BNE to the next word, which jumps.
            assert_eq!(verify_jal(code, BASE + 4, 0, destination), Ok(5000));
        }
    }
}

#[test]
fn rejects_invalid_control_transfers() {
    let mut code = [0x13_u32; 4].map(u32::to_le_bytes);
    let intended = BASE + 8;
    let original = 0x0080_006f_u32;
    code[0] = original.to_le_bytes();
    assert_eq!(verify_jal(code.as_flattened(), BASE, 0, intended), Ok(8));
    for (mask, expected) in [
        (0x0040_0000, Rejection::WrongTarget(BASE + 12)),
        (0x0000_0080, Rejection::WrongRd(1)),
        (0x0180_0000, Rejection::TargetBounds(BASE + 16)),
        (0x00e0_0000, Rejection::TargetAlignment(BASE + 6)),
        (0x0000_007c, Rejection::Opcode(0x13)),
    ] {
        let word = original ^ mask;
        code[0] = word.to_le_bytes();
        let observed = verify_jal(code.as_flattened(), BASE, 0, intended);
        println!(
            "Mutated JAL {word:#010x}, decoding={:?}, verification={observed:?}",
            decode_jal(word, BASE)
        );
        assert_eq!(observed, Err(expected));
    }

    code[0] = original.to_le_bytes();
    let code = code.as_flattened();
    assert_eq!(
        verify_jal(code, BASE + 2, 0, intended),
        Err(Rejection::SiteAlignment(BASE + 2))
    );
    assert_eq!(
        verify_jal(&code[..3], BASE, 0, intended),
        Err(Rejection::SiteBounds(BASE))
    );
    assert_eq!(
        verify_jal(&code[..11], BASE, 0, intended),
        Err(Rejection::TargetBounds(intended))
    );

    let mut code = [0x13_u32; 3].map(u32::to_le_bytes);
    code[0] = 0x0062_9463_u32.to_le_bytes();
    let observed = verify_branch(code.as_flattened(), BASE, 0, intended);
    println!("BNE at expected BEQ site: {observed:?}");
    assert_eq!(observed, Err(Rejection::WrongBranchFields(t0_t1(1))));
    code[0] = original.to_le_bytes();
    let observed = verify_branch(code.as_flattened(), BASE, 0, intended);
    println!("JAL at expected BEQ site: {observed:?}");
    assert_eq!(observed, Err(Rejection::Opcode(0x6f)));
}

#[test]
fn rejects_missing_jal_sign_bit_and_address_wraparound() {
    let mut code = [0x0000_0013_u32, 0xffdf_f06f].map(u32::to_le_bytes);
    assert_eq!(verify_jal(code.as_flattened(), BASE + 4, 0, BASE), Ok(-4));
    let corrupted = 0x7fdf_f06f_u32;
    code[1] = corrupted.to_le_bytes();
    let (rd, actual) = decode_jal(corrupted, BASE + 4).unwrap();
    let observed = verify_jal(code.as_flattened(), BASE + 4, 0, BASE);
    println!("LF-027 fixture {corrupted:#010x}, rd=x{rd}, {actual:?}, verification={observed:?}");
    assert_eq!(rd, 0);
    assert_eq!(
        actual,
        Destination {
            displacement: 1_048_572,
            pc: 0x8010_0000,
        }
    );
    assert_eq!(observed, Err(Rejection::TargetBounds(0x8010_0000)));

    for (word, site) in [(0xffdf_f06f, 0), (0x0080_006f, u64::MAX - 3)] {
        let observed = decode_jal(word, site);
        println!("JAL {word:#010x} at {site:#018x}, decoding={observed:?}");
        assert_eq!(observed, Err(Rejection::AddressOverflow));
    }
}

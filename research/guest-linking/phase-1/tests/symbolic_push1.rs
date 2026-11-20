use std::{collections::BTreeSet, fmt::Write};

use asm_rs::{Arch, Assembler, AssemblyResult};

const ENTRY: u64 = 0x8000_0000;
const EXIT: u64 = 0x800f_fffc;
const NORMAL: &str = "normal_exit";
const OVERFLOW: &str = "push1_stack_overflow_exit";

// The instructions mirror the publishers in ferreth-compiler/src/compiler.rs.
const PUBLISHERS: &str = r#"normal_exit:
    addi t1, s0, -2048
    addi t1, t1, -2048
    sd s1, 16(t1)
    sd zero, 8(t1)
    addi t0, zero, 1
    sd t0, 0(t1)
    jalr zero, -4(t1)
push1_stack_overflow_exit:
    addi t1, s0, -2048
    addi t1, t1, -2048
    sd s1, 16(t1)
    addi t0, zero, 1
    sd t0, 8(t1)
    addi t0, zero, 2
    sd t0, 0(t1)
    jalr zero, -4(t1)
"#;

fn assemble_pushes(immediates: &[u8]) -> (String, AssemblyResult) {
    let mut source = String::new();
    for (index, immediate) in immediates.iter().enumerate() {
        // Keep the production PUSH1 instructions and name their destinations.
        write!(
            source,
            r#"guest_push1_{index}:
    lui t0, 8
    add t0, s0, t0
    bgeu s1, t0, push1_overflow_{index}
    addi t0, zero, {immediate}
    sd t0, 0(s1)
    sd zero, 8(s1)
    sd zero, 16(s1)
    sd zero, 24(s1)
    addi s1, s1, 32
    jal zero, push1_continue_{index}
push1_overflow_{index}:
    jal zero, {OVERFLOW}
push1_continue_{index}:
"#,
        )
        .unwrap();
    }
    source.push_str(PUBLISHERS);

    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.base_address(ENTRY).emit(&source).unwrap();
    let result = assembler.finish().unwrap();
    (source, result)
}

fn read_word(code: &[u8], pc: u64) -> u32 {
    assert!(pc.is_multiple_of(4), "unaligned PC {pc:#x}");
    let offset = usize::try_from(pc.checked_sub(ENTRY).unwrap()).unwrap();
    u32::from_le_bytes(code[offset..][..4].try_into().unwrap())
}

// Use Phase 1.3's instruction-bit decoding and the instruction's own PC.
fn verify_target(code: &[u8], site: u64, immediate: u32, bits: u32, expected: u64) {
    let shift = 32 - bits;
    let displacement = ((immediate << shift) as i32) >> shift;
    let target = site.checked_add_signed(i64::from(displacement)).unwrap();
    read_word(code, target);
    assert_eq!(target, expected, "target of instruction at {site:#x}");
    let word = read_word(code, site);
    println!("{site:#010x}: {word:08x} ({displacement:+}) -> {target:#010x}");
}

fn verify_jal(code: &[u8], site: u64, expected: u64) {
    let word = read_word(code, site);
    assert_eq!(word & 0xfff, 0x06f, "expected JAL x0 at {site:#x}");
    let immediate = ((word >> 31) << 20)
        | (((word >> 21) & 0x3ff) << 1)
        | (((word >> 20) & 1) << 11)
        | (word & 0xff000);
    verify_target(code, site, immediate, 21, expected);
}

fn verify_pushes(result: &AssemblyResult, immediates: &[u8]) {
    let code = result.bytes();
    assert_eq!(result.base_address(), ENTRY);
    assert!(code.len().is_multiple_of(4));
    let end = ENTRY + code.len() as u64;
    assert!(end <= EXIT);

    let pc = |name: &str| result.label_address(name).unwrap();
    let normal = pc(NORMAL);
    let overflow = pc(OVERFLOW);
    assert_eq!(pc("guest_push1_0"), ENTRY);
    assert_eq!(result.labels().len(), immediates.len() * 3 + 2);
    let mut local_sites = BTreeSet::new();

    for (index, immediate) in immediates.iter().copied().enumerate() {
        let entry = pc(&format!("guest_push1_{index}"));
        let trampoline = pc(&format!("push1_overflow_{index}"));
        let continuation = pc(&format!("push1_continue_{index}"));
        let next = if index + 1 == immediates.len() {
            normal
        } else {
            pc(&format!("guest_push1_{}", index + 1))
        };
        assert!(local_sites.insert(entry));
        assert!(local_sites.insert(trampoline));
        assert_eq!(trampoline, entry + 40);
        assert_eq!(continuation, trampoline + 4);
        assert_eq!(continuation, next);

        // Check the unchanged limit setup and stack writes around the transfers.
        assert_eq!(read_word(code, entry), 0x0000_82b7);
        assert_eq!(read_word(code, entry + 4), 0x0054_02b3);
        for (slot, expected) in [
            (u32::from(immediate) << 20) | 0x293,
            0x0054_b023,
            0x0004_b423,
            0x0004_b823,
            0x0004_bc23,
            0x0204_8493,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(read_word(code, entry + 12 + slot as u64 * 4), expected);
        }

        let check = entry + 8;
        let branch = read_word(code, check);
        // BGEU s1 (x9), t0 (x5). Its untaken path starts at entry + 12.
        assert_eq!(branch & 0x01ff_f07f, 0x0054_f063);
        let displacement = ((branch >> 31) << 12)
            | (((branch >> 25) & 0x3f) << 5)
            | (((branch >> 8) & 0xf) << 1)
            | (((branch >> 7) & 1) << 11);
        verify_target(code, check, displacement, 13, trampoline);
        verify_jal(code, trampoline - 4, continuation);
        verify_jal(code, trampoline, overflow);
    }

    // Check the fixed publisher ends. Under ADR-001, s0 = 0x80101000.
    // The two ADDIs set t1 to 0x80100000 before JALR x0, -4(t1).
    assert_eq!(overflow, normal + 28);
    assert_eq!(end, overflow + 32);
    for (publisher, return_site) in [(normal, overflow - 4), (overflow, end - 4)] {
        assert_eq!(read_word(code, publisher), 0x8004_0313);
        assert_eq!(read_word(code, publisher + 4), 0x8003_0313);
        assert_eq!(read_word(code, return_site), 0xffc3_0067);
    }
    assert_eq!(0x8010_1000_u64 - 2048 - 2048 - 4, EXIT);
    println!(
        "{} bytes, normal={normal:#010x}, overflow={overflow:#010x}",
        code.len()
    );
}

#[test]
fn one_push1() {
    let (_, result) = assemble_pushes(&[2]);
    verify_pushes(&result, &[2]);
}

#[test]
fn two_push1_specimen() {
    let (source, result) = assemble_pushes(&[2, 3]);
    verify_pushes(&result, &[2, 3]);
    println!("Assembly:\n{source}");
    let code = result.bytes();
    print!("Bytes: ");
    for byte in code {
        print!("{byte:02x}");
    }
    println!();
    for (index, bytes) in code.as_chunks::<4>().0.iter().enumerate() {
        println!(
            "{:08x}: {:08x}",
            ENTRY + index as u64 * 4,
            u32::from_le_bytes(*bytes)
        );
    }
}

#[test]
fn repeated_push1_with_distinct_immediates() {
    let immediates = [0, 1, 127, 128, 255];
    let (_, result) = assemble_pushes(&immediates);
    verify_pushes(&result, &immediates);
}

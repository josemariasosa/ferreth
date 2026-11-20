use asm_rs::{Arch, AsmError, Assembler, AssemblyResult, RelocKind};

const BASE: u64 = 0x8000_0000;
const NOP: &str = "addi zero, zero, 0\n";
const JAL: &str = "jal zero, destination\n";

fn assemble(source: &str) -> Result<AssemblyResult, AsmError> {
    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.base_address(BASE).emit(source)?;
    assembler.finish()
}

fn word_at(result: &AssemblyResult, offset: usize) -> u32 {
    u32::from_le_bytes(result.bytes()[offset..offset + 4].try_into().unwrap())
}

fn assert_relocation(result: &AssemblyResult, offset: usize, kind: RelocKind) {
    let [relocation] = result.relocations() else {
        panic!(
            "expected one applied relocation, got {:?}",
            result.relocations()
        );
    };
    assert_eq!(relocation.offset, offset);
    assert_eq!(relocation.size, 4);
    assert_eq!(relocation.label, "destination");
    assert_eq!(relocation.kind, kind);
    assert_eq!(relocation.addend, 0);
}

fn assert_jal(
    result: &AssemblyResult,
    site_offset: usize,
    target_offset: usize,
    expected_word: u32,
) {
    let site = result.label_address("site").unwrap();
    let target = result.label_address("destination").unwrap();
    let displacement = target as i64 - site as i64;
    let word = word_at(result, site_offset);
    println!(
        "JAL site={site:#010x}, destination={target:#010x}, displacement={displacement:+}, \
         word={word:#010x}, size={}, relocations={:?}",
        result.len(),
        result.relocations()
    );

    assert_eq!(result.base_address(), BASE);
    assert_eq!(site, BASE + site_offset as u64);
    assert_eq!(target, BASE + target_offset as u64);
    assert_eq!(result.len(), site_offset.max(target_offset) + 4);
    assert_eq!(word, expected_word, "JAL displacement {displacement:+}");
    assert_relocation(result, site_offset, RelocKind::RvJal20);
}

#[test]
fn symbolic_jal_forward() -> Result<(), AsmError> {
    // Leave one instruction before the site to exercise a nonzero relocation offset.
    // Expected words are literal J-type fixtures for rd=x0.
    for (displacement, word) in [
        (8, 0x0080_006f),
        (2044, 0x7fc0_006f),
        (2048, 0x0010_006f),
        (2056, 0x0090_006f),
        (4096, 0x0000_106f),
    ] {
        let padding = NOP.repeat(displacement / 4 - 1);
        let source = format!("{NOP}site:\n{JAL}{padding}destination:\n{NOP}");
        assert_jal(&assemble(&source)?, 4, 4 + displacement, word);
    }
    Ok(())
}

#[test]
fn symbolic_jal_backward() -> Result<(), AsmError> {
    for (distance, word) in [(4, 0xffdf_f06f), (4096, 0x800f_f06f)] {
        let padding = NOP.repeat(distance / 4);
        let source = format!("destination:\n{padding}site:\n{JAL}");
        assert_jal(&assemble(&source)?, distance, 0, word);
    }
    Ok(())
}

#[test]
fn symbolic_jal_signed_boundaries() -> Result<(), AsmError> {
    // Use a single data fragment for the MiB padding to keep source and work bounded.
    let source = "destination:\n.space 1048576\nsite:\njal zero, destination\n";
    assert_jal(&assemble(source)?, 1_048_576, 0, 0x8000_006f);

    for (target_offset, word) in [(1_048_572, 0x7fdf_f06f), (1_048_574, 0x7fff_f06f)] {
        let padding = target_offset - 4;
        let source = format!("site:\n{JAL}.space {padding}\ndestination:\n{NOP}");
        assert_jal(&assemble(&source)?, 0, target_offset, word);
    }

    // First even offsets outside either endpoint, plus the first four-byte
    // aligned offset below the minimum.
    for (site_offset, target_offset) in [(0, 1_048_576), (1_048_578, 0), (1_048_580, 0)] {
        let source = if site_offset == 0 {
            let padding = target_offset - 4;
            format!("site:\n{JAL}.space {padding}\ndestination:\n{NOP}")
        } else {
            format!("destination:\n.space {site_offset}\nsite:\n{JAL}")
        };
        let displacement = rejected_jal_layout(&source, site_offset, target_offset)?;
        let error = assemble(&source).expect_err("out-of-range JAL must fail");
        println!("{error:?}\n{error}");
        assert!(matches!(
            error,
            AsmError::BranchOutOfRange { label, disp, max: 1_048_574, .. }
                if label == "destination" && disp == displacement
        ));
    }
    Ok(())
}

// Failed assembly has no result table. Substitute a same-width instruction
// at the JAL site to inspect both labels without applying a relocation.
fn rejected_jal_layout(
    source: &str,
    site_offset: u64,
    target_offset: u64,
) -> Result<i64, AsmError> {
    let control = assemble(&source.replacen(JAL, NOP, 1))?;
    let site = control.label_address("site").unwrap();
    let target = control.label_address("destination").unwrap();
    let displacement = target as i64 - site as i64;
    println!("Layout control: site={site:#010x}, destination={target:#010x}, displacement={displacement:+}");
    assert_eq!(site, BASE + site_offset);
    assert_eq!(target, BASE + target_offset);
    assert!(control.relocations().is_empty());
    Ok(displacement)
}

#[test]
fn symbolic_jal_alignment() -> Result<(), AsmError> {
    let aligned = format!("site:\n{JAL}destination:\n{NOP}");
    assert_jal(&assemble(&aligned)?, 0, 4, 0x0040_006f);

    let halfword_aligned = format!("site:\n{JAL}.byte 0, 0\ndestination:\n{NOP}");
    assert_jal(&assemble(&halfword_aligned)?, 0, 6, 0x0060_006f);

    let odd = format!("site:\n{JAL}.byte 0\ndestination:\n{NOP}");
    let displacement = rejected_jal_layout(&odd, 0, 5)?;
    let error = assemble(&odd).expect_err("odd JAL displacement must fail");
    println!("{error:?}\n{error}");
    assert!(matches!(
        error,
        AsmError::MisalignedBranchTarget { label, disp, alignment: 2, .. }
            if label == "destination" && disp == displacement
    ));
    Ok(())
}

#[test]
fn named_symbol_errors() {
    // Successful forward and backward named references are covered above.
    let mut undefined = Assembler::new(Arch::Rv64);
    undefined.base_address(BASE).emit(JAL).unwrap();
    let error = undefined.finish().expect_err("undefined label must fail");
    println!("finish: {error:?}\n{error}");
    assert!(matches!(
        error,
        AsmError::UndefinedLabel { label, span }
            if label == "destination" && span == Default::default()
    ));

    let mut duplicate = Assembler::new(Arch::Rv64);
    duplicate.base_address(BASE);
    let source = format!("destination:\n{NOP}destination:\n{NOP}");
    let error = match duplicate.emit(&source) {
        Ok(_) => panic!("duplicate label must fail during emit"),
        Err(error) => error,
    };
    println!("emit: {error:?}\n{error}");
    assert!(matches!(
        error,
        AsmError::DuplicateLabel { label, span, first_span }
            if label == "destination"
                && (first_span.line, first_span.col) == (1, 1)
                && (span.line, span.col) == (3, 1)
    ));
}

#[test]
fn symbolic_conditional_branch_relaxation() -> Result<(), AsmError> {
    for (padding_count, expected_words, kind) in [
        (1, &[0x0062_8463][..], RelocKind::RvBranch12),
        (1249, &[0x0062_9463, 0x3880_106f][..], RelocKind::RvJal20),
    ] {
        let padding = NOP.repeat(padding_count);
        let source =
            format!("site:\nbeq t0, t1, destination\nfallthrough:\n{padding}destination:\n{NOP}");
        let result = assemble(&source)?;
        let site = result.label_address("site").unwrap();
        let fallthrough = result.label_address("fallthrough").unwrap();
        let target = result.label_address("destination").unwrap();
        let end = (fallthrough - BASE) as usize;
        let words: Vec<u32> = (0..end)
            .step_by(4)
            .map(|offset| word_at(&result, offset))
            .collect();
        println!(
            "BEQ padding={padding_count} instructions, size={}, site={site:#010x}, \
             fallthrough={fallthrough:#010x}, destination={target:#010x}, \
             control-flow instructions={}, words={words:08x?}, relocations={:?}",
            result.len(),
            words.len(),
            result.relocations()
        );
        assert_eq!(site, BASE);
        assert_eq!(fallthrough - site, (expected_words.len() * 4) as u64);
        assert_eq!(target - fallthrough, (padding_count * 4) as u64);
        assert_eq!(result.len(), end + (padding_count + 1) * 4);
        assert_eq!(words, expected_words);
        assert_relocation(&result, end - 4, kind);
    }
    Ok(())
}

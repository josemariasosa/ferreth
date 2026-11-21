//! Final-byte checks for the fixed layouts of the three current guest linkers.

use super::memory::{ENTRY_PC, RV64_INSTRUCTION_SIZE};

const BRANCH: u32 = 0x63;
const JALR: u32 = 0x67;
const JAL: u32 = 0x6f;

// RV64 register numbers and branch funct3 values in the generator contracts.
const ZERO: u32 = 0;
const RA: u32 = 1;
const T0: u32 = 5;
const S0: u32 = 8;
const S1: u32 = 9;
const BLTU: u32 = 6;
const BGEU: u32 = 7;

const STACK_DECREMENT: u32 = 0xfe04_8493; // addi s1, s1, -32
const LEAF_RETURN: u32 = 0x0000_8067; // jalr zero, 0(ra)

#[allow(dead_code, reason = "Phase 2.4 legacy-layout and rollback coverage")]
#[derive(Clone, Copy)]
pub(crate) enum Fragment {
    Push1,
    Pop,
    Dup1,
    Arithmetic,
    SymbolicArithmetic(ArithmeticCore),
    Stop,
}

#[derive(Clone, Copy)]
pub(crate) enum ArithmeticCore {
    Add,
    Sub,
}

impl ArithmeticCore {
    fn words(self) -> &'static [u32; 34] {
        // Standalone core words at the accepted Phase 2.3b.1 revision.
        // See phase-2.3c-symbolic-arithmetic.md for the byte comparison.
        match self {
            Self::Add => &[
                0x0000_0e13,
                0x0005_3283,
                0x0005_b303,
                0x0062_83b3,
                0x0053_beb3,
                0x01c3_83b3,
                0x01c3_bf33,
                0x01ee_ee33,
                0x0076_3023,
                0x0085_3283,
                0x0085_b303,
                0x0062_83b3,
                0x0053_beb3,
                0x01c3_83b3,
                0x01c3_bf33,
                0x01ee_ee33,
                0x0076_3423,
                0x0105_3283,
                0x0105_b303,
                0x0062_83b3,
                0x0053_beb3,
                0x01c3_83b3,
                0x01c3_bf33,
                0x01ee_ee33,
                0x0076_3823,
                0x0185_3283,
                0x0185_b303,
                0x0062_83b3,
                0x0053_beb3,
                0x01c3_83b3,
                0x01c3_bf33,
                0x01ee_ee33,
                0x0076_3c23,
                0x0000_8067,
            ],
            Self::Sub => &[
                0x0000_0e13,
                0x0005_3283,
                0x0005_b303,
                0x4062_83b3,
                0x0062_beb3,
                0x41c3_82b3,
                0x0053_bf33,
                0x01ee_ee33,
                0x0056_3023,
                0x0085_3283,
                0x0085_b303,
                0x4062_83b3,
                0x0062_beb3,
                0x41c3_82b3,
                0x0053_bf33,
                0x01ee_ee33,
                0x0056_3423,
                0x0105_3283,
                0x0105_b303,
                0x4062_83b3,
                0x0062_beb3,
                0x41c3_82b3,
                0x0053_bf33,
                0x01ee_ee33,
                0x0056_3823,
                0x0185_3283,
                0x0185_b303,
                0x4062_83b3,
                0x0062_beb3,
                0x41c3_82b3,
                0x0053_bf33,
                0x01ee_ee33,
                0x0056_3c23,
                0x0000_8067,
            ],
        }
    }
}

#[allow(dead_code, reason = "Phase 2.4 legacy-layout and rollback coverage")]
#[derive(Clone, Copy)]
pub(crate) enum PublisherLayout {
    PushOnly,
    Symbolic,
    SymbolicNormalOverflow,
    SymbolicNormalUnderflow,
    SymbolicNormalUnderflowOverflow,
    Mixed,
    IsolatedArithmetic,
}

/// Call after the ABI code-region check. Fragment kinds come from the requested
/// operations. Publisher PCs are the exact metadata the linker will return.
pub(crate) fn verify_final_image(
    bytes: &[u8],
    fragments: impl IntoIterator<Item = Fragment>,
    layout: PublisherLayout,
    publisher_pcs: &[u64],
) -> Result<(), String> {
    let (normal, overflow, underflow) = match (layout, publisher_pcs) {
        (PublisherLayout::PushOnly, &[normal, overflow]) => (normal, Some(overflow), None),
        (PublisherLayout::Symbolic, &[normal]) => (normal, None, None),
        (PublisherLayout::SymbolicNormalOverflow, &[normal, overflow]) => {
            (normal, Some(overflow), None)
        }
        (PublisherLayout::SymbolicNormalUnderflow, &[normal, underflow]) => {
            (normal, None, Some(underflow))
        }
        (PublisherLayout::SymbolicNormalUnderflowOverflow, &[normal, underflow, overflow]) => {
            (normal, Some(overflow), Some(underflow))
        }
        (PublisherLayout::Mixed, &[normal, overflow, underflow]) => {
            (normal, Some(overflow), Some(underflow))
        }
        (PublisherLayout::IsolatedArithmetic, &[normal, underflow]) => {
            (normal, None, Some(underflow))
        }
        _ => return Err("incorrect publisher count for guest layout".to_owned()),
    };
    let image = Image {
        bytes,
        base_pc: ENTRY_PC,
    };
    let mut pc = ENTRY_PC;
    for fragment in fragments {
        pc = fragment.verify(&image, pc, normal, overflow, underflow)?;
    }
    let underflow = underflow.map(|entry| (entry, Publisher::Underflow));
    let overflow = overflow.map(|entry| (entry, Publisher::Overflow));
    let exceptional = match layout {
        PublisherLayout::Symbolic
        | PublisherLayout::SymbolicNormalOverflow
        | PublisherLayout::SymbolicNormalUnderflow
        | PublisherLayout::SymbolicNormalUnderflowOverflow => [underflow, overflow],
        _ => [overflow, underflow],
    };
    for (entry, publisher) in
        std::iter::once((normal, Publisher::Normal)).chain(exceptional.into_iter().flatten())
    {
        if entry != pc {
            return Err(format!(
                "{publisher:?} publisher PC {entry:#x}, expected {pc:#x}"
            ));
        }
        pc = publisher.verify(&image, pc)?;
    }
    if pc - ENTRY_PC != bytes.len() as u64 {
        return Err(format!(
            "unexpected image bytes after publisher end at {pc:#x}"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transfer {
    Jal { rd: u32 },
    Branch { funct3: u32, rs1: u32, rs2: u32 },
    LeafReturn,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DeclaredTransfer {
    pub(crate) site_pc: u64,
    pub(crate) target_pc: u64,
    pub(crate) kind: Transfer,
}

/// Checks every control transfer in the fragment area against fragment-owned
/// declarations. The fixed-layout verifier remains the independent completeness
/// check for required sites, destinations, and arithmetic core identity.
pub(crate) fn verify_declared_transfers(
    bytes: &[u8],
    publisher_start: u64,
    declarations: &[DeclaredTransfer],
) -> Result<(), String> {
    let image = Image {
        bytes,
        base_pc: ENTRY_PC,
    };
    if !publisher_start.is_multiple_of(RV64_INSTRUCTION_SIZE) {
        return Err(format!(
            "publisher start {publisher_start:#x} is not four-byte aligned"
        ));
    }
    let body_bytes = publisher_start
        .checked_sub(ENTRY_PC)
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| format!("publisher start {publisher_start:#x} precedes the image"))?;
    if body_bytes > bytes.len() || !body_bytes.is_multiple_of(4) {
        return Err(format!(
            "publisher start {publisher_start:#x} is outside the executable image"
        ));
    }

    let mut declarations = declarations.to_vec();
    declarations.sort_by_key(|declaration| declaration.site_pc);
    if declarations
        .windows(2)
        .any(|pair| pair[0].site_pc == pair[1].site_pc)
    {
        return Err("duplicate symbolic transfer declaration".to_owned());
    }
    for declaration in &declarations {
        if declaration.site_pc < ENTRY_PC || declaration.site_pc >= publisher_start {
            return Err(format!(
                "declared transfer site {:#x} is outside the fragment area",
                declaration.site_pc
            ));
        }
        if !declaration.site_pc.is_multiple_of(RV64_INSTRUCTION_SIZE) {
            return Err(format!(
                "declared transfer site {:#x} is not four-byte aligned",
                declaration.site_pc
            ));
        }
    }

    let mut next_declaration = 0;
    for offset in (0..body_bytes).step_by(4) {
        let pc = ENTRY_PC + offset as u64;
        let word = image.word(pc)?;
        if matches!(word & 0x7f, JAL | BRANCH | JALR) {
            let Some(declaration) = declarations.get(next_declaration) else {
                return Err(format!(
                    "unexpected control transfer {word:#010x} at {pc:#x}"
                ));
            };
            if declaration.site_pc != pc {
                return Err(format!(
                    "unexpected control transfer {word:#010x} at {pc:#x}"
                ));
            }
            image.verify_transfer(pc, word, &declaration.kind, declaration.target_pc)?;
            next_declaration += 1;
        } else if word & 3 != 3 || word & 0x1f == 0x1f {
            return Err(format!("unsupported instruction width at {pc:#x}"));
        }
    }
    if let Some(declaration) = declarations.get(next_declaration) {
        return Err(format!(
            "declared transfer site {:#x} is not an encoded control transfer",
            declaration.site_pc
        ));
    }
    Ok(())
}

impl Fragment {
    fn verify(
        self,
        image: &Image<'_>,
        start: u64,
        normal: u64,
        overflow: Option<u64>,
        underflow: Option<u64>,
    ) -> Result<u64, String> {
        // Word indices come from the opcode generators. ADD/SUB each embed a
        // 34-word leaf core after the nine-word wrapper.
        let words = match self {
            Self::Push1 => 11,
            Self::Pop => 4,
            Self::Dup1 => 16,
            Self::Arithmetic | Self::SymbolicArithmetic(_) => 9 + 34,
            Self::Stop => 1,
        };
        let next = pc_at_word(start, words)?;
        let overflow = || overflow.ok_or("fragment requires an overflow publisher");
        let underflow = || underflow.ok_or("fragment requires an underflow publisher");
        match self {
            Self::Push1 => image.verify_block(
                start,
                words,
                &[
                    Site::branch(2, BGEU, S1, T0, pc_at_word(start, 10)?),
                    Site::jal(9, ZERO, next),
                    Site::jal(10, ZERO, overflow()?),
                ],
            ),
            Self::Pop => image.verify_block(
                start,
                words,
                &[
                    Site::branch(0, BGEU, S0, S1, pc_at_word(start, 3)?),
                    Site::jal(2, ZERO, next),
                    Site::jal(3, ZERO, underflow()?),
                ],
            ),
            Self::Dup1 => image.verify_block(
                start,
                words,
                &[
                    Site::branch(0, BGEU, S0, S1, pc_at_word(start, 14)?),
                    Site::branch(3, BGEU, S1, T0, pc_at_word(start, 15)?),
                    Site::jal(13, ZERO, next),
                    Site::jal(14, ZERO, underflow()?),
                    Site::jal(15, ZERO, overflow()?),
                ],
            ),
            Self::Arithmetic | Self::SymbolicArithmetic(_) => image.verify_block(
                start,
                words,
                &[
                    Site::branch(1, BLTU, S1, T0, pc_at_word(start, 8)?),
                    Site::jal(5, RA, pc_at_word(start, 9)?),
                    Site::fixed(6, STACK_DECREMENT), // The call's PC+4.
                    Site::jal(7, ZERO, next),
                    Site::jal(8, ZERO, underflow()?),
                    Site::fixed(words - 1, LEAF_RETURN),
                ],
            ),
            Self::Stop if next == normal => {
                image.verify_block(start, words, &[Site::jal(0, ZERO, normal)])
            }
            Self::Stop => Err(format!(
                "STOP at {start:#x} must precede the normal publisher"
            )),
        }?;
        if let Self::SymbolicArithmetic(core) = self {
            // This contract is independent of resolved labels and declarations.
            // Require the preflight and pointer setup as well as the leaf bytes.
            for (index, expected) in [
                (0, 0x0404_0293), // addi t0, s0, 64
                (2, 0xfe04_8513), // addi a0, s1, -32
                (3, 0xfc04_8593), // addi a1, s1, -64
                (4, 0xfc04_8613), // addi a2, s1, -64
            ]
            .into_iter()
            .chain((9..).zip(core.words().iter().copied()))
            {
                let pc = pc_at_word(start, index)?;
                let word = image.word(pc)?;
                if word != expected {
                    return Err(format!(
                        "arithmetic at {pc:#x}: word {word:#010x}, expected {expected:#010x}"
                    ));
                }
            }
        }
        Ok(next)
    }
}

#[derive(Debug)]
enum Publisher {
    Normal,
    Overflow,
    Underflow,
}

impl Publisher {
    fn verify(self, image: &Image<'_>, start: u64) -> Result<u64, String> {
        // ADR-001: t1 = s0 - 4096, then store final_s1 at metadata + 16.
        const PREFIX: [u32; 3] = [0x8004_0313, 0x8003_0313, 0x0093_3823];
        // Code at metadata + 8, status at metadata + 0, then the sentinel JALR.
        let tail: &[u32] = match self {
            Self::Normal => &[0x0003_3423, 0x0010_0293, 0x0053_3023, 0xffc3_0067],
            Self::Overflow => &[
                0x0010_0293,
                0x0053_3423,
                0x0020_0293,
                0x0053_3023,
                0xffc3_0067,
            ],
            Self::Underflow => &[
                0x0020_0293,
                0x0053_3423,
                0x0020_0293,
                0x0053_3023,
                0xffc3_0067,
            ],
        };
        for (index, &expected) in PREFIX.iter().chain(tail).enumerate() {
            let pc = pc_at_word(start, index as u64)?;
            let word = image.word(pc)?;
            if word != expected {
                return Err(format!(
                    "{self:?} publisher at {pc:#x}: word {word:#010x}, expected {expected:#010x}"
                ));
            }
        }
        pc_at_word(start, (PREFIX.len() + tail.len()) as u64)
    }
}

struct Site {
    index: u64,
    instruction: Expected,
}

enum Expected {
    Transfer { kind: Transfer, target_pc: u64 },
    Fixed(u32),
}

impl Site {
    fn fixed(index: u64, word: u32) -> Self {
        Self {
            index,
            instruction: Expected::Fixed(word),
        }
    }

    fn jal(index: u64, rd: u32, target_pc: u64) -> Self {
        Self {
            index,
            instruction: Expected::Transfer {
                kind: Transfer::Jal { rd },
                target_pc,
            },
        }
    }

    fn branch(index: u64, funct3: u32, rs1: u32, rs2: u32, target_pc: u64) -> Self {
        Self {
            index,
            instruction: Expected::Transfer {
                kind: Transfer::Branch { funct3, rs1, rs2 },
                target_pc,
            },
        }
    }
}

struct Image<'a> {
    bytes: &'a [u8],
    base_pc: u64,
}

impl Image<'_> {
    fn word(&self, pc: u64) -> Result<u32, String> {
        if !pc.is_multiple_of(RV64_INSTRUCTION_SIZE) {
            return Err(format!("PC {pc:#x} is not four-byte aligned"));
        }
        pc.checked_sub(self.base_pc)
            .and_then(|offset| usize::try_from(offset).ok())
            .and_then(|offset| self.bytes.get(offset..))
            .and_then(|bytes| bytes.first_chunk().copied())
            .map(u32::from_le_bytes)
            .ok_or_else(|| format!("full word at PC {pc:#x} is outside the executable image"))
    }

    fn verify_block(&self, start: u64, words: u64, sites: &[Site]) -> Result<(), String> {
        if sites.windows(2).any(|pair| pair[0].index >= pair[1].index)
            || sites.last().is_some_and(|site| site.index >= words)
        {
            return Err(format!(
                "invalid or duplicated site declaration in fragment at {start:#x}"
            ));
        }
        let mut sites = sites.iter().peekable();
        for index in 0..words {
            let pc = pc_at_word(start, index)?;
            let word = self.word(pc)?;
            if let Some(site) = sites.next_if(|site| site.index == index) {
                match &site.instruction {
                    Expected::Transfer { kind, target_pc } => {
                        self.verify_transfer(pc, word, kind, *target_pc)?
                    }
                    Expected::Fixed(expected) if word == *expected => (),
                    Expected::Fixed(expected) => {
                        return Err(format!(
                            "at {pc:#x}: word {word:#010x}, expected {expected:#010x}"
                        ))
                    }
                }
            } else if matches!(word & 0x7f, JAL | BRANCH | JALR) {
                return Err(format!(
                    "unexpected control transfer {word:#010x} at {pc:#x}"
                ));
            } else if word & 3 != 3 || word & 0x1f == 0x1f {
                return Err(format!("unsupported instruction width at {pc:#x}"));
            }
        }
        Ok(())
    }

    fn verify_transfer(
        &self,
        pc: u64,
        word: u32,
        kind: &Transfer,
        target_pc: u64,
    ) -> Result<(), String> {
        if *kind == Transfer::LeafReturn {
            if word != LEAF_RETURN {
                return Err(format!(
                    "at {pc:#x}: word {word:#010x}, expected leaf return"
                ));
            }
            let call_pc = target_pc
                .checked_sub(RV64_INSTRUCTION_SIZE)
                .ok_or_else(|| format!("invalid leaf return anchor {target_pc:#x}"))?;
            let (call, displacement) = decode_transfer(call_pc, self.word(call_pc)?)?;
            let leaf = call_pc
                .checked_add_signed(i64::from(displacement))
                .ok_or_else(|| format!("invalid leaf call at {call_pc:#x}"))?;
            if call != (Transfer::Jal { rd: RA })
                || pc_at_word(leaf, 33)? != pc
                || self.word(target_pc)? != STACK_DECREMENT
            {
                return Err(format!("invalid leaf return anchor {target_pc:#x}"));
            }
            // The fixed layout also identifies the required entry and call sites
            // without using these declarations.
            return Ok(());
        }
        let (actual, displacement) = decode_transfer(pc, word)?;
        if actual != *kind {
            return Err(format!("at {pc:#x}: {actual:?}, expected {kind:?}"));
        }
        let target = pc
            .checked_add_signed(i64::from(displacement))
            .ok_or_else(|| {
                format!("target address overflow at {pc:#x} with displacement {displacement}")
            })?;
        self.word(target)
            .map_err(|error| format!("transfer at {pc:#x}: {error}"))?;
        if target != target_pc {
            return Err(format!(
                "transfer at {pc:#x} targets {target:#x}, expected {target_pc:#x}"
            ));
        }
        Ok(())
    }
}

fn pc_at_word(start: u64, index: u64) -> Result<u64, String> {
    index
        .checked_mul(RV64_INSTRUCTION_SIZE)
        .and_then(|offset| start.checked_add(offset))
        .ok_or_else(|| format!("instruction address overflow from {start:#x} at word {index}"))
}

fn decode_transfer(pc: u64, word: u32) -> Result<(Transfer, i32), String> {
    let opcode = word & 0x7f;
    let (kind, immediate, bits) = match opcode {
        JAL => (
            Transfer::Jal {
                rd: (word >> 7) & 0x1f,
            },
            ((word >> 31) << 20)
                | (word & 0xff000)
                | (((word >> 20) & 1) << 11)
                | (((word >> 21) & 0x3ff) << 1),
            21,
        ),
        BRANCH => (
            Transfer::Branch {
                funct3: (word >> 12) & 7,
                rs1: (word >> 15) & 0x1f,
                rs2: (word >> 20) & 0x1f,
            },
            ((word >> 31) << 12)
                | (((word >> 7) & 1) << 11)
                | (((word >> 25) & 0x3f) << 5)
                | (((word >> 8) & 0xf) << 1),
            13,
        ),
        _ => {
            return Err(format!(
                "unexpected opcode {opcode:#04x} at declared transfer {pc:#x}"
            ))
        }
    };
    let shift = 32 - bits;
    Ok((kind, ((immediate << shift) as i32) >> shift))
}

#[cfg(test)]
mod tests;

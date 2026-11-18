use std::fmt::Write;

use revm::primitives::U256;

pub mod benchmark;
pub mod compiler;
pub mod disassembler;
pub mod execution;
pub mod orchestrator;
pub mod rv64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    Stop,
    Push(U256),
    Add,
    Sub,
    Pop,
    Dup1,
}

/// One decoded EVM instruction with source metadata for compiler diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodedInstruction {
    Stop {
        offset: usize,
    },
    Push {
        value: U256,
        opcode: u8,
        offset: usize,
    },
    Add {
        offset: usize,
    },
    Sub {
        offset: usize,
    },
    Pop {
        offset: usize,
    },
    Dup1 {
        offset: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    UnsupportedOpcode {
        offset: usize,
        opcode: u8,
    },
    /// Retained for source compatibility. The decoder now zero-pads missing
    /// PUSH immediate bytes and no longer emits this error.
    TruncatedPush {
        offset: usize,
        opcode: u8,
        expected: usize,
        available: usize,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedOpcode { offset, opcode } => {
                write!(f, "unsupported opcode 0x{opcode:02x} at byte offset {offset}")
            }
            Self::TruncatedPush {
                offset,
                opcode,
                expected,
                available,
            } => write!(
                f,
                "truncated PUSH opcode 0x{opcode:02x} at byte offset {offset}: expected {expected} immediate bytes, found {available}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes the strict STOP, PUSH1-PUSH32, ADD, SUB, POP, and DUP1 research subset.
///
/// Decoding ends at the first `STOP`, so bytes after it are unreachable and
/// intentionally not interpreted as EVM instructions.
pub fn decode(bytecode: &[u8]) -> Result<Vec<Instruction>, DecodeError> {
    decode_detailed(bytecode).map(|instructions| {
        instructions
            .into_iter()
            .map(|instruction| match instruction {
                DecodedInstruction::Stop { .. } => Instruction::Stop,
                DecodedInstruction::Push { value, .. } => Instruction::Push(value),
                DecodedInstruction::Add { .. } => Instruction::Add,
                DecodedInstruction::Sub { .. } => Instruction::Sub,
                DecodedInstruction::Pop { .. } => Instruction::Pop,
                DecodedInstruction::Dup1 { .. } => Instruction::Dup1,
            })
            .collect()
    })
}

/// Decodes bytecode once while retaining opcode and byte-offset metadata.
/// A PUSH that extends past the end of bytecode fills its missing trailing
/// immediate bytes with zeros.
pub fn decode_detailed(bytecode: &[u8]) -> Result<Vec<DecodedInstruction>, DecodeError> {
    let mut instructions = Vec::new();
    let mut pc = 0;
    while pc < bytecode.len() {
        let offset = pc;
        let opcode = bytecode[pc];
        pc += 1;
        let instruction = match opcode {
            0x00 => DecodedInstruction::Stop { offset },
            0x01 => DecodedInstruction::Add { offset },
            0x03 => DecodedInstruction::Sub { offset },
            0x50 => DecodedInstruction::Pop { offset },
            0x80 => DecodedInstruction::Dup1 { offset },
            0x60..=0x7f => {
                let width = push_width(opcode).expect("0x60..=0x7f are PUSH opcodes");
                let available = width.min(bytecode.len() - pc);
                let mut bytes = [0; 32];
                let immediate_start = bytes.len() - width;
                bytes[immediate_start..immediate_start + available]
                    .copy_from_slice(&bytecode[pc..pc + available]);
                pc += width;
                DecodedInstruction::Push {
                    value: U256::from_be_bytes(bytes),
                    opcode,
                    offset,
                }
            }
            _ => return Err(DecodeError::UnsupportedOpcode { offset, opcode }),
        };
        let is_stop = matches!(instruction, DecodedInstruction::Stop { .. });
        instructions.push(instruction);
        if is_stop {
            break;
        }
    }
    Ok(instructions)
}

/// Immediate byte count for `PUSH1` (0x60) through `PUSH32` (0x7f).
pub(crate) const fn push_width(opcode: u8) -> Option<usize> {
    match opcode {
        0x60..=0x7f => Some((opcode - 0x5f) as usize),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Add,
    Sub,
}

impl Opcode {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::Add),
            0x03 => Some(Self::Sub),
            _ => None,
        }
    }

    pub fn assembly(&self) -> String {
        match self {
            Self::Add => generate_add(),
            Self::Sub => generate_sub(),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Add => "ADD",
            Self::Sub => "SUB",
        }
    }

    pub const fn evm_byte(self) -> u8 {
        match self {
            Self::Add => 0x01,
            Self::Sub => 0x03,
        }
    }
}

/// Adds little-endian limbs modulo 2^256.
pub fn add_u256(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut result = [0; 4];
    let mut carry = false;
    for i in 0..4 {
        let (sum, carry_a) = a[i].overflowing_add(b[i]);
        let (sum, carry_b) = sum.overflowing_add(u64::from(carry));
        result[i] = sum;
        carry = carry_a || carry_b;
    }
    result
}

/// Subtracts little-endian limbs modulo 2^256.
pub fn sub_u256(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut result = [0; 4];
    let mut borrow = false;
    for i in 0..4 {
        let (difference, borrow_a) = a[i].overflowing_sub(b[i]);
        let (difference, borrow_b) = difference.overflowing_sub(u64::from(borrow));
        result[i] = difference;
        borrow = borrow_a || borrow_b;
    }
    result
}

fn generate_add() -> String {
    let mut asm = String::from(
        ".text\n\
         .p2align 2\n\
         .globl evm_add\n\
         .type evm_add, @function\n\
         evm_add:\n\
         \x20   li t3, 0\n",
    );
    for offset in [0, 8, 16, 24] {
        writeln!(
            asm,
            "    ld t0, {offset}(a0)\n\
             \x20   ld t1, {offset}(a1)\n\
             \x20   add t2, t0, t1\n\
             \x20   sltu t4, t2, t0\n\
             \x20   add t2, t2, t3\n\
             \x20   sltu t5, t2, t3\n\
             \x20   or t3, t4, t5\n\
             \x20   sd t2, {offset}(a2)"
        )
        .expect("writing to a String cannot fail");
    }
    asm.push_str("    ret\n.size evm_add, .-evm_add\n");
    asm
}

/// Generates `evm_sub`, using a0/a1 as input pointers and a2 as the output
/// pointer. Inputs and output are four little-endian u64 limbs. It clobbers
/// caller-saved t0-t5, uses no stack, and discards the final borrow.
fn generate_sub() -> String {
    let mut asm = String::from(
        ".text\n\
         .p2align 2\n\
         .globl evm_sub\n\
         .type evm_sub, @function\n\
         evm_sub:\n\
         \x20   li t3, 0\n",
    );
    for offset in [0, 8, 16, 24] {
        writeln!(
            asm,
            "    ld t0, {offset}(a0)\n\
             \x20   ld t1, {offset}(a1)\n\
             \x20   sub t2, t0, t1\n\
             \x20   sltu t4, t0, t1\n\
             \x20   sub t0, t2, t3\n\
             \x20   sltu t5, t2, t0\n\
             \x20   or t3, t4, t5\n\
             \x20   sd t0, {offset}(a2)"
        )
        .expect("writing to a String cannot fail");
    }
    asm.push_str("    ret\n.size evm_sub, .-evm_sub\n");
    asm
}

#[cfg(test)]
mod tests {
    use super::{
        add_u256, decode, decode_detailed, sub_u256, DecodeError, DecodedInstruction, Instruction,
        Opcode,
    };
    use revm::primitives::U256;

    #[test]
    fn decodes_push_widths_and_opcode_like_immediates() {
        assert_eq!(decode(&[]), Ok(vec![]));
        assert_eq!(
            decode(&[0x60, 0x03, 0x01]),
            Ok(vec![Instruction::Push(U256::from(3)), Instruction::Add])
        );
        for width in 1..=32 {
            let immediate: Vec<u8> = (0..width)
                .map(|index| [0x01, 0x03, 0x60, 0x7f, 0x02][index % 5])
                .collect();
            let mut bytecode = vec![0x5f + width as u8];
            bytecode.extend_from_slice(&immediate);
            bytecode.push(0x03);
            let mut bytes = [0; 32];
            bytes[32 - width..].copy_from_slice(&immediate);
            assert_eq!(
                decode(&bytecode),
                Ok(vec![
                    Instruction::Push(U256::from_be_bytes(bytes)),
                    Instruction::Sub
                ]),
                "PUSH{width}"
            );
        }
    }

    #[test]
    fn decodes_stop_and_ignores_unreachable_trailing_bytes() {
        assert_eq!(decode(&[0x00]), Ok(vec![Instruction::Stop]));
        assert_eq!(
            decode(&[0x60, 0x05, 0x00, 0x02]),
            Ok(vec![Instruction::Push(U256::from(5)), Instruction::Stop])
        );
    }

    #[test]
    fn decodes_dup1_and_ignores_it_after_stop() {
        assert_eq!(decode(&[0x80]), Ok(vec![Instruction::Dup1]));
        assert_eq!(
            decode(&[0x60, 0x05, 0x00, 0x80]),
            Ok(vec![Instruction::Push(U256::from(5)), Instruction::Stop])
        );
    }

    #[test]
    fn rejects_unsupported_opcodes_at_byte_offsets() {
        let unsupported = decode(&[0x61, 0x02, 0x01, 0x02]).unwrap_err();
        assert_eq!(
            unsupported,
            DecodeError::UnsupportedOpcode {
                offset: 3,
                opcode: 0x02
            }
        );
        assert_eq!(
            unsupported.to_string(),
            "unsupported opcode 0x02 at byte offset 3"
        );
    }

    #[test]
    fn decodes_truncated_push_immediates_with_trailing_zeros() {
        for (bytecode, value) in [
            (&[0x60][..], 0x00u16),
            (&[0x61, 0xab][..], 0xab00),
            (&[0x61][..], 0x0000),
            (&[0x61, 0xab, 0xcd][..], 0xabcd),
        ] {
            assert_eq!(
                decode_detailed(bytecode),
                Ok(vec![DecodedInstruction::Push {
                    value: U256::from(value),
                    opcode: bytecode[0],
                    offset: 0,
                }]),
                "bytecode={bytecode:02x?}"
            );
        }

        let mut wide_value = [0; 32];
        wide_value[..2].copy_from_slice(&[0x01, 0x03]);
        assert_eq!(
            decode_detailed(&[0x60, 0x7f, 0x7f, 0x01, 0x03]),
            Ok(vec![
                DecodedInstruction::Push {
                    value: U256::from(0x7fu8),
                    opcode: 0x60,
                    offset: 0,
                },
                DecodedInstruction::Push {
                    value: U256::from_be_bytes(wide_value),
                    opcode: 0x7f,
                    offset: 2,
                },
            ])
        );
    }

    #[test]
    fn identifies_add_opcode() {
        assert_eq!(Opcode::from_byte(0x01), Some(Opcode::Add));
        assert_eq!(Opcode::from_byte(0x03), Some(Opcode::Sub));
        assert_eq!(Opcode::from_byte(0x00), None);
    }

    #[test]
    fn zero_operands() {
        assert_eq!(add_u256([0; 4], [0; 4]), [0; 4]);
    }

    #[test]
    fn simple_addition() {
        assert_eq!(add_u256([5, 0, 0, 0], [7, 0, 0, 0]), [12, 0, 0, 0]);
    }

    #[test]
    fn carry_across_limb_boundary() {
        assert_eq!(add_u256([u64::MAX, 0, 0, 0], [1, 0, 0, 0]), [0, 1, 0, 0]);
    }

    #[test]
    fn carry_propagates_through_all_limbs() {
        assert_eq!(
            add_u256([u64::MAX, u64::MAX, u64::MAX, 0], [1, 0, 0, 0]),
            [0, 0, 0, 1]
        );
    }

    #[test]
    fn overflow_wraps_to_zero() {
        assert_eq!(add_u256([u64::MAX; 4], [1, 0, 0, 0]), [0; 4]);
    }

    #[test]
    fn randomized_against_u128_limb_arithmetic() {
        let mut state = 0x1234_5678_9abc_def0_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..10_000 {
            let a = std::array::from_fn(|_| next());
            let b = std::array::from_fn(|_| next());
            let mut expected = [0; 4];
            let mut carry = 0_u128;
            for i in 0..4 {
                let sum = u128::from(a[i]) + u128::from(b[i]) + carry;
                expected[i] = sum as u64;
                carry = sum >> 64;
            }
            assert_eq!(add_u256(a, b), expected, "a={a:?}, b={b:?}");
        }
    }

    #[test]
    fn subtraction_borrows_across_limb_boundary() {
        assert_eq!(sub_u256([0, 1, 0, 0], [1, 0, 0, 0]), [u64::MAX, 0, 0, 0]);
    }

    #[test]
    fn subtraction_borrows_through_all_limbs() {
        assert_eq!(
            sub_u256([0, 0, 0, 1], [1, 0, 0, 0]),
            [u64::MAX, u64::MAX, u64::MAX, 0]
        );
    }

    #[test]
    fn subtraction_wraps_modulo_u256() {
        assert_eq!(sub_u256([0; 4], [1, 0, 0, 0]), [u64::MAX; 4]);
    }

    #[test]
    fn randomized_subtraction_against_u128_limb_arithmetic() {
        let mut state = 0x0fed_cba9_8765_4321_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..10_000 {
            let a = std::array::from_fn(|_| next());
            let b = std::array::from_fn(|_| next());
            let mut expected = [0; 4];
            let mut borrow = 0_u128;
            for i in 0..4 {
                let minuend = u128::from(a[i]);
                let subtrahend = u128::from(b[i]) + borrow;
                expected[i] = minuend.wrapping_sub(subtrahend) as u64;
                borrow = u128::from(minuend < subtrahend);
            }
            assert_eq!(sub_u256(a, b), expected, "a={a:?}, b={b:?}");
        }
    }
}

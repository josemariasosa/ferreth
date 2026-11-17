//! Static EVM bytecode disassembly.
//!
//! This walks the whole byte stream (including bytes after `STOP`) using the
//! same PUSH width rule as [`crate::decode_detailed`]. It does not execute,
//! simulate a stack, or imply that the RV64 compiler supports an opcode.

use std::fmt::Write;

use crate::push_width;

/// Classification of one disassembled byte span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisassembledKind {
    /// A defined Cancun opcode. `immediate` is `Some` for complete PUSH1-PUSH32.
    Known {
        mnemonic: String,
        immediate: Option<Vec<u8>>,
    },
    /// A PUSH whose immediate runs past the end of the bytecode. `immediate`
    /// holds only the available bytes; `missing` counts the absent ones.
    TruncatedPush {
        mnemonic: String,
        immediate: Vec<u8>,
        missing: usize,
    },
    /// A byte that is not a defined opcode in Cancun.
    Undefined,
}

/// One instruction span: `bytes` is exactly `bytecode[pc..pc + bytes.len()]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisassembledInstruction {
    pub pc: usize,
    pub bytes: Vec<u8>,
    pub kind: DisassembledKind,
}

impl DisassembledInstruction {
    /// Human-readable opcode column text.
    pub fn text(&self) -> String {
        match &self.kind {
            DisassembledKind::Known {
                mnemonic,
                immediate: None,
            } => mnemonic.clone(),
            DisassembledKind::Known {
                mnemonic,
                immediate: Some(value),
            } => format!("{mnemonic} 0x{}", hex(value)),
            DisassembledKind::TruncatedPush {
                mnemonic,
                immediate,
                missing,
            } => {
                let plural = if *missing == 1 { "" } else { "s" };
                let shown = if immediate.is_empty() {
                    String::new()
                } else {
                    format!(" 0x{}", hex(immediate))
                };
                format!("{mnemonic}{shown} (truncated: missing {missing} immediate byte{plural})")
            }
            DisassembledKind::Undefined => format!("UNKNOWN (0x{:02x})", self.bytes[0]),
        }
    }
}

/// Disassembles every byte of `bytecode`; no byte is skipped or covered twice.
pub fn disassemble(bytecode: &[u8]) -> Vec<DisassembledInstruction> {
    let mut instructions = Vec::new();
    let mut pc = 0;
    while pc < bytecode.len() {
        let opcode = bytecode[pc];
        let start = pc;
        pc += 1;
        let kind = match (mnemonic(opcode), push_width(opcode)) {
            (None, _) => DisassembledKind::Undefined,
            (Some(mnemonic), None) => DisassembledKind::Known {
                mnemonic,
                immediate: None,
            },
            (Some(mnemonic), Some(width)) => {
                let available = (bytecode.len() - pc).min(width);
                let immediate = bytecode[pc..pc + available].to_vec();
                pc += available;
                if available < width {
                    DisassembledKind::TruncatedPush {
                        mnemonic,
                        immediate,
                        missing: width - available,
                    }
                } else {
                    DisassembledKind::Known {
                        mnemonic,
                        immediate: Some(immediate),
                    }
                }
            }
        };
        instructions.push(DisassembledInstruction {
            pc: start,
            bytes: bytecode[start..pc].to_vec(),
            kind,
        });
    }
    instructions
}

/// Renders an aligned `PC  BYTES  OPCODE` table.
pub fn format_disassembly(instructions: &[DisassembledInstruction]) -> String {
    let rows: Vec<_> = instructions
        .iter()
        .map(|instruction| (instruction.pc, hex(&instruction.bytes), instruction.text()))
        .collect();
    let pc_width = rows
        .last()
        .map_or(4, |(pc, _, _)| format!("{pc:04x}").len())
        + 2;
    let bytes_width = rows
        .iter()
        .map(|(_, bytes, _)| bytes.len())
        .chain([10])
        .max()
        .unwrap_or(5);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<pc_width$}  {:<bytes_width$}  OPCODE",
        "PC", "BYTES"
    );
    for (pc, bytes, text) in rows {
        let _ = writeln!(
            out,
            "{:<pc_width$}  {bytes:<bytes_width$}  {text}",
            format!("0x{pc:04x}")
        );
    }
    out
}

/// Parses hex without a prefix; accepts upper or lower case.
pub fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    if let Some((index, character)) = input
        .char_indices()
        .find(|(_, character)| !character.is_ascii_hexdigit())
    {
        return Err(format!(
            "invalid hexadecimal character {character:?} at position {index}"
        ));
    }
    if !input.len().is_multiple_of(2) {
        return Err(format!(
            "hexadecimal input has odd length {}; expected whole bytes",
            input.len()
        ));
    }
    Ok((0..input.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&input[index..index + 2], 16).expect("validated hex"))
        .collect())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Canonical Cancun mnemonic, or `None` for an undefined byte.
fn mnemonic(opcode: u8) -> Option<String> {
    let fixed = match opcode {
        0x00 => "STOP",
        0x01 => "ADD",
        0x02 => "MUL",
        0x03 => "SUB",
        0x04 => "DIV",
        0x05 => "SDIV",
        0x06 => "MOD",
        0x07 => "SMOD",
        0x08 => "ADDMOD",
        0x09 => "MULMOD",
        0x0a => "EXP",
        0x0b => "SIGNEXTEND",
        0x10 => "LT",
        0x11 => "GT",
        0x12 => "SLT",
        0x13 => "SGT",
        0x14 => "EQ",
        0x15 => "ISZERO",
        0x16 => "AND",
        0x17 => "OR",
        0x18 => "XOR",
        0x19 => "NOT",
        0x1a => "BYTE",
        0x1b => "SHL",
        0x1c => "SHR",
        0x1d => "SAR",
        0x20 => "KECCAK256",
        0x30 => "ADDRESS",
        0x31 => "BALANCE",
        0x32 => "ORIGIN",
        0x33 => "CALLER",
        0x34 => "CALLVALUE",
        0x35 => "CALLDATALOAD",
        0x36 => "CALLDATASIZE",
        0x37 => "CALLDATACOPY",
        0x38 => "CODESIZE",
        0x39 => "CODECOPY",
        0x3a => "GASPRICE",
        0x3b => "EXTCODESIZE",
        0x3c => "EXTCODECOPY",
        0x3d => "RETURNDATASIZE",
        0x3e => "RETURNDATACOPY",
        0x3f => "EXTCODEHASH",
        0x40 => "BLOCKHASH",
        0x41 => "COINBASE",
        0x42 => "TIMESTAMP",
        0x43 => "NUMBER",
        0x44 => "PREVRANDAO",
        0x45 => "GASLIMIT",
        0x46 => "CHAINID",
        0x47 => "SELFBALANCE",
        0x48 => "BASEFEE",
        0x49 => "BLOBHASH",
        0x4a => "BLOBBASEFEE",
        0x50 => "POP",
        0x51 => "MLOAD",
        0x52 => "MSTORE",
        0x53 => "MSTORE8",
        0x54 => "SLOAD",
        0x55 => "SSTORE",
        0x56 => "JUMP",
        0x57 => "JUMPI",
        0x58 => "PC",
        0x59 => "MSIZE",
        0x5a => "GAS",
        0x5b => "JUMPDEST",
        0x5c => "TLOAD",
        0x5d => "TSTORE",
        0x5e => "MCOPY",
        0x5f => "PUSH0",
        0xf0 => "CREATE",
        0xf1 => "CALL",
        0xf2 => "CALLCODE",
        0xf3 => "RETURN",
        0xf4 => "DELEGATECALL",
        0xf5 => "CREATE2",
        0xfa => "STATICCALL",
        0xfd => "REVERT",
        0xfe => "INVALID",
        0xff => "SELFDESTRUCT",
        0x60..=0x7f => return Some(format!("PUSH{}", opcode - 0x5f)),
        0x80..=0x8f => return Some(format!("DUP{}", opcode - 0x7f)),
        0x90..=0x9f => return Some(format!("SWAP{}", opcode - 0x8f)),
        0xa0..=0xa4 => return Some(format!("LOG{}", opcode - 0xa0)),
        _ => return None,
    };
    Some(fixed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(bytecode: &[u8]) -> Vec<(usize, String)> {
        disassemble(bytecode)
            .iter()
            .map(|i| (i.pc, i.text()))
            .collect()
    }

    fn assert_exact_cover(bytecode: &[u8]) {
        let rebuilt: Vec<u8> = disassemble(bytecode)
            .iter()
            .flat_map(|i| i.bytes.clone())
            .collect();
        assert_eq!(rebuilt, bytecode);
        let mut next = 0;
        for instruction in disassemble(bytecode) {
            assert_eq!(instruction.pc, next);
            next += instruction.bytes.len();
        }
        assert_eq!(next, bytecode.len());
    }

    #[test]
    fn empty_bytecode_has_header_only() {
        assert!(disassemble(&[]).is_empty());
        assert_eq!(format_disassembly(&[]), "PC      BYTES       OPCODE\n");
    }

    #[test]
    fn simple_opcodes() {
        assert_eq!(
            texts(&[0x00, 0x01, 0x03, 0x50]),
            vec![
                (0, "STOP".into()),
                (1, "ADD".into()),
                (2, "SUB".into()),
                (3, "POP".into())
            ]
        );
    }

    #[test]
    fn formats_multiple_instructions_with_byte_offsets() {
        let bytecode = [0x60, 0x05, 0x60, 0x03, 0x50];
        assert_eq!(
            format_disassembly(&disassemble(&bytecode)),
            "PC      BYTES       OPCODE\n\
             0x0000  6005        PUSH1 0x05\n\
             0x0002  6003        PUSH1 0x03\n\
             0x0004  50          POP\n"
        );
        assert_exact_cover(&bytecode);
    }

    #[test]
    fn shows_instructions_after_stop() {
        let bytecode = [0x60, 0x05, 0x00, 0x60, 0x09];
        assert_eq!(
            texts(&bytecode),
            vec![
                (0, "PUSH1 0x05".into()),
                (2, "STOP".into()),
                (3, "PUSH1 0x09".into())
            ]
        );
        assert_exact_cover(&bytecode);
    }

    #[test]
    fn push2_and_push32_consume_their_immediates() {
        assert_eq!(
            texts(&[0x61, 0x01, 0x02, 0x01]),
            vec![(0, "PUSH2 0x0102".into()), (3, "ADD".into())]
        );
        let mut bytecode = vec![0x7f];
        bytecode.extend(1..=32u8);
        bytecode.push(0x50);
        let listing = disassemble(&bytecode);
        assert_eq!(listing.len(), 2);
        assert_eq!(listing[0].bytes.len(), 33);
        assert_eq!(listing[1].pc, 33);
        assert_exact_cover(&bytecode);
    }

    #[test]
    fn opcode_like_immediates_are_not_instructions() {
        let bytecode = [0x62, 0x00, 0x50, 0x60, 0x00];
        assert_eq!(
            texts(&bytecode),
            vec![(0, "PUSH3 0x005060".into()), (4, "STOP".into())]
        );
        assert_exact_cover(&bytecode);
    }

    #[test]
    fn truncated_push_reports_available_and_missing_bytes() {
        let listing = disassemble(&[0x61, 0x01]);
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].bytes, vec![0x61, 0x01]);
        assert_eq!(
            listing[0].text(),
            "PUSH2 0x01 (truncated: missing 1 immediate byte)"
        );
        assert_eq!(
            texts(&[0x7f]),
            vec![(
                0,
                "PUSH32 (truncated: missing 32 immediate bytes)".to_string()
            )]
        );
    }

    #[test]
    fn distinguishes_known_uncompiled_invalid_and_undefined() {
        assert_eq!(
            texts(&[0x02, 0xfe, 0x0c, 0x5f]),
            vec![
                (0, "MUL".into()),
                (1, "INVALID".into()),
                (2, "UNKNOWN (0x0c)".into()),
                (3, "PUSH0".into())
            ]
        );
    }

    #[test]
    fn parses_hex_case_insensitively_and_rejects_malformed_input() {
        assert_eq!(parse_hex("6005"), Ok(vec![0x60, 0x05]));
        assert_eq!(parse_hex("7F0aFf"), Ok(vec![0x7f, 0x0a, 0xff]));
        assert_eq!(parse_hex(""), Ok(vec![]));
        assert!(parse_hex("600").unwrap_err().contains("odd length"));
        assert!(parse_hex("60zz").unwrap_err().contains("position 2"));
        assert!(parse_hex("0x60").unwrap_err().contains("'x'"));
        assert!(parse_hex("+560").is_err());
    }
}

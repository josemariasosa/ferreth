use std::fmt::Write;

use asm_rs::{assemble, Arch};

use crate::{
    compiler::symbolic::{ExternalDeclaration, LocalDeclaration, PublisherRole, SymbolicFragment},
    rv64::verify::{Fragment, Transfer},
};

use super::gas::{emit_static_charge, STATIC_CHARGE_WORDS};
use crate::rv64::codegen::GeneratedRv64;

/// Generates one stack-aware RV64 implementation of EVM `PUSH1`.
///
/// `s1` is checked against the ABI stack limit before any stack store or stack
/// pointer update.  On success, it writes the immediate in limb zero, clears
/// the remaining three limbs, and advances `s1` by one 32-byte EVM word.  It
/// neither reads nor writes `sp`, and it preserves `s0`.
pub fn generate_guest_push1(immediate: u8) -> GeneratedRv64 {
    let mut assembly = String::from("guest_push1:\n");
    emit_push1_body(&mut assembly, immediate, "32", "8");
    assembly.push_str("    jal zero, 0\n");
    let machine_code = assemble(&assembly, Arch::Rv64)
        .expect("fixed PUSH1 RV64 assembly must assemble into base RV64 instructions");
    assert_eq!(machine_code.len(), 11 * 4, "PUSH1 encoding changed width");

    GeneratedRv64 {
        assembly,
        machine_code,
        link_metadata: None,
    }
}

pub(crate) fn generate_symbolic_push1(immediate: u8, index: usize) -> SymbolicFragment {
    let entry = format!("guest_push1_{index}");
    let overflow = format!("push1_overflow_{index}");
    let continuation = format!("push1_continue_{index}");
    let mut assembly = String::new();
    writeln!(assembly, "{entry}:").unwrap();
    let (gas_check, oog) = emit_static_charge(&mut assembly, &entry, 3);
    emit_push1_body(&mut assembly, immediate, &overflow, &continuation);
    writeln!(
        &mut assembly,
        "{overflow}:\n    jal zero, {}\n{continuation}:",
        PublisherRole::Overflow.label()
    )
    .unwrap();
    SymbolicFragment::new(
        assembly,
        entry,
        vec![
            ExternalDeclaration {
                site_word: STATIC_CHARGE_WORDS + 10,
                role: PublisherRole::Overflow,
                kind: Transfer::Jal { rd: 0 },
            },
            oog,
        ],
        vec![
            gas_check,
            LocalDeclaration {
                site_word: STATIC_CHARGE_WORDS + 2,
                target_symbol: overflow,
                kind: Transfer::Branch {
                    funct3: 7,
                    rs1: 9,
                    rs2: 5,
                },
            },
            LocalDeclaration {
                site_word: STATIC_CHARGE_WORDS + 9,
                target_symbol: continuation,
                kind: Transfer::Jal { rd: 0 },
            },
        ],
        Fragment::MeteredPush1,
    )
}

fn emit_push1_body(assembly: &mut String, immediate: u8, overflow: &str, continuation: &str) {
    // Deriving the limit from s0 avoids materializing the 0x8010_9000 guest
    // address, whose 32-bit sign extension is inconvenient on RV64.
    writeln!(
        assembly,
        "    lui t0, 8\n\
             \x20   add t0, s0, t0\n\
             \x20   bgeu s1, t0, {overflow}\n\
             \x20   addi t0, zero, {immediate}\n\
             \x20   sd t0, 0(s1)\n\
             \x20   sd zero, 8(s1)\n\
             \x20   sd zero, 16(s1)\n\
             \x20   sd zero, 24(s1)\n\
             \x20   addi s1, s1, 32\n\
             \x20   jal zero, {continuation}"
    )
    .unwrap();
}

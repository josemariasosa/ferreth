use std::fmt::Write;

use crate::{
    compiler::symbolic::{ExternalDeclaration, LocalDeclaration, PublisherRole, SymbolicFragment},
    rv64::{
        guest::GUEST_METADATA_LIVE_REMAINING_OFFSET,
        memory::{EVM_STACK_SIZE, EVM_WORD_SIZE},
        verify::{Fragment, Transfer},
    },
};

pub(super) const STATIC_CHARGE_WORDS: u64 = 8;

/// Emit at fragment entry, before operand setup. Only t0–t2 are clobbered.
/// The cost must fit a positive signed 12-bit immediate.
pub(super) fn emit_static_charge(
    assembly: &mut String,
    entry: &str,
    cost: u16,
) -> (LocalDeclaration, ExternalDeclaration) {
    assert!((1..=2047).contains(&cost));
    let sufficient = format!("{entry}_gas_sufficient");
    let oog = PublisherRole::OutOfGas;
    let remaining = GUEST_METADATA_LIVE_REMAINING_OFFSET;
    writeln!(
        assembly,
        "    addi t1, s0, -2048\n\
         \x20   addi t1, t1, -2048\n\
         \x20   ld t0, {remaining}(t1)\n\
         \x20   addi t2, zero, {cost}\n\
         \x20   bgeu t0, t2, {sufficient}\n\
         \x20   jal zero, {}\n\
         {sufficient}:\n\
         \x20   sub t0, t0, t2\n\
         \x20   sd t0, {remaining}(t1)",
        oog.label()
    )
    .unwrap();
    (
        LocalDeclaration {
            site_word: 4,
            target_symbol: sufficient,
            kind: Transfer::Branch {
                funct3: 7,
                rs1: 5,
                rs2: 7,
            },
        },
        ExternalDeclaration {
            site_word: 5,
            role: oog,
            kind: Transfer::Jal { rd: 0 },
        },
    )
}

pub(crate) fn generate_symbolic_gas(index: usize, continuation: String) -> SymbolicFragment {
    let entry = format!("guest_gas_{index}");
    let overflow = format!("{entry}_overflow");
    let mut assembly = format!("{entry}:\n");
    let (gas_check, oog) = emit_static_charge(&mut assembly, &entry, 2);

    // The capacity guard replaces t0 but leaves the metadata base in t1.
    let remaining = GUEST_METADATA_LIVE_REMAINING_OFFSET;
    let stack_pages = EVM_STACK_SIZE >> 12;
    writeln!(
        assembly,
        "    lui t0, {stack_pages}\n\
         \x20   add t0, s0, t0\n\
         \x20   bgeu s1, t0, {overflow}\n\
         \x20   ld t0, {remaining}(t1)\n\
         \x20   sd t0, 0(s1)\n\
         \x20   sd zero, 8(s1)\n\
         \x20   sd zero, 16(s1)\n\
         \x20   sd zero, 24(s1)\n\
         \x20   addi s1, s1, {EVM_WORD_SIZE}\n\
         \x20   jal zero, {continuation}\n\
         {overflow}:\n\
         \x20   jal zero, {}",
        PublisherRole::Overflow.label()
    )
    .unwrap();

    SymbolicFragment::new(
        assembly,
        entry,
        vec![
            oog,
            ExternalDeclaration {
                site_word: STATIC_CHARGE_WORDS + 10,
                role: PublisherRole::Overflow,
                kind: Transfer::Jal { rd: 0 },
            },
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
        Fragment::Gas,
    )
}

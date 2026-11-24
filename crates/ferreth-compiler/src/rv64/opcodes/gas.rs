use std::fmt::Write;

use crate::{
    compiler::symbolic::{ExternalDeclaration, LocalDeclaration, PublisherRole},
    rv64::{guest::GUEST_METADATA_LIVE_REMAINING_OFFSET, verify::Transfer},
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

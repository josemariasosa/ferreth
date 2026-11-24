//! Reusable stack-aware RV64 implementation of EVM `POP`.
//!
//! POP only changes the guest next-free stack pointer. Its underflow path is
//! an initially self-jumping trampoline, patched by the program linker to the
//! shared guest stack-underflow publisher.

use asm_rs::{assemble, Arch};

use crate::rv64::codegen::GeneratedRv64;
use crate::{
    compiler::symbolic::{ExternalDeclaration, LocalDeclaration, PublisherRole, SymbolicFragment},
    rv64::verify::{Fragment, Transfer},
};
use std::fmt::Write;

use super::gas::{emit_static_charge, STATIC_CHARGE_WORDS};

/// Generates one stack-aware RV64 implementation of EVM `POP`.
///
/// The generated code checks that `s1 > s0` before retiring the top word.
/// Success decrements `s1` by exactly one EVM word without reading or writing
/// stack memory; failure leaves both ABI stack registers unchanged and enters
/// a patchable `jal zero, 0` underflow trampoline.
pub fn generate_guest_pop() -> GeneratedRv64 {
    let assembly = [
        "guest_pop:\n",
        "    bgeu s0, s1, guest_pop_underflow\n",
        "    addi s1, s1, -32\n",
        "    jal zero, 8\n",
        "guest_pop_underflow:\n",
        "    jal zero, 0\n",
    ]
    .concat();
    let machine_code = assemble(&assembly, Arch::Rv64)
        .expect("fixed POP RV64 assembly must assemble into base RV64 instructions");
    assert_eq!(machine_code.len(), 4 * 4, "POP encoding changed width");

    GeneratedRv64 {
        assembly,
        machine_code,
        link_metadata: None,
    }
}

pub(crate) fn generate_symbolic_pop(index: usize, continuation: String) -> SymbolicFragment {
    let entry = format!("guest_pop_{index}");
    let underflow = format!("pop_underflow_{index}");
    let mut assembly = String::new();
    writeln!(assembly, "{entry}:").unwrap();
    let (gas_check, oog) = emit_static_charge(&mut assembly, &entry, 2);
    writeln!(
        assembly,
        "    bgeu s0, s1, {underflow}\n\
         \x20   addi s1, s1, -32\n\
         \x20   jal zero, {continuation}\n\
         {underflow}:\n\
         \x20   jal zero, {}",
        PublisherRole::Underflow.label()
    )
    .unwrap();

    let mut external_references = vec![
        oog,
        ExternalDeclaration {
            site_word: STATIC_CHARGE_WORDS + 3,
            role: PublisherRole::Underflow,
            kind: Transfer::Jal { rd: 0 },
        },
    ];
    let mut local_references = vec![
        gas_check,
        LocalDeclaration {
            site_word: STATIC_CHARGE_WORDS,
            target_symbol: underflow,
            kind: Transfer::Branch {
                funct3: 7,
                rs1: 8,
                rs2: 9,
            },
        },
    ];
    let continuation_site = STATIC_CHARGE_WORDS + 2;
    if continuation == PublisherRole::Normal.label() {
        external_references.push(ExternalDeclaration {
            site_word: continuation_site,
            role: PublisherRole::Normal,
            kind: Transfer::Jal { rd: 0 },
        });
    } else {
        local_references.push(LocalDeclaration {
            site_word: continuation_site,
            target_symbol: continuation,
            kind: Transfer::Jal { rd: 0 },
        });
    }

    SymbolicFragment::new(
        assembly,
        entry,
        external_references,
        local_references,
        Fragment::MeteredPop,
    )
}

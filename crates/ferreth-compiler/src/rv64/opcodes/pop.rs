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

    let continuation_reference = if continuation == PublisherRole::Normal.label() {
        vec![ExternalDeclaration {
            site_word: 2,
            role: PublisherRole::Normal,
            kind: Transfer::Jal { rd: 0 },
        }]
    } else {
        Vec::new()
    };
    let mut external_references = vec![ExternalDeclaration {
        site_word: 3,
        role: PublisherRole::Underflow,
        kind: Transfer::Jal { rd: 0 },
    }];
    external_references.extend(continuation_reference);

    let local_references = if continuation == PublisherRole::Normal.label() {
        vec![LocalDeclaration {
            site_word: 0,
            target_symbol: underflow,
            kind: Transfer::Branch {
                funct3: 7,
                rs1: 8,
                rs2: 9,
            },
        }]
    } else {
        vec![
            LocalDeclaration {
                site_word: 0,
                target_symbol: underflow,
                kind: Transfer::Branch {
                    funct3: 7,
                    rs1: 8,
                    rs2: 9,
                },
            },
            LocalDeclaration {
                site_word: 2,
                target_symbol: continuation,
                kind: Transfer::Jal { rd: 0 },
            },
        ]
    };

    SymbolicFragment::new(
        assembly,
        entry,
        external_references,
        local_references,
        Fragment::Pop,
    )
}

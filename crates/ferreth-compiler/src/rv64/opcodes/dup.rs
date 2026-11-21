//! Reusable stack-aware RV64 implementation of EVM `DUP1`.
//!
//! The body has separate patchable underflow and overflow trampolines. A
//! successful execution copies the top U256 word to the next-free slot and
//! advances `s1`.

use asm_rs::{assemble, Arch};

use crate::rv64::codegen::GeneratedRv64;
use crate::{
    compiler::symbolic::{ExternalDeclaration, LocalDeclaration, PublisherRole, SymbolicFragment},
    rv64::verify::{Fragment, Transfer},
};
use std::fmt::Write;

/// Generates one stack-aware RV64 implementation of EVM `DUP1`.
///
/// The generated code checks underflow and capacity before touching stack
/// memory. It copies all four little-endian limbs, preserves `s0` and the
/// original top word, and increments `s1` only after all stores complete.
pub fn generate_guest_dup1() -> GeneratedRv64 {
    let assembly = [
        "guest_dup1:\n",
        "    bgeu s0, s1, guest_dup1_underflow\n",
        "    lui t0, 8\n",
        "    add t0, s0, t0\n",
        "    bgeu s1, t0, guest_dup1_overflow\n",
        "    ld t1, -32(s1)\n",
        "    ld t2, -24(s1)\n",
        "    ld t3, -16(s1)\n",
        "    ld t4, -8(s1)\n",
        "    sd t1, 0(s1)\n",
        "    sd t2, 8(s1)\n",
        "    sd t3, 16(s1)\n",
        "    sd t4, 24(s1)\n",
        "    addi s1, s1, 32\n",
        "    jal zero, 12\n",
        "guest_dup1_underflow:\n",
        "    jal zero, 0\n",
        "guest_dup1_overflow:\n",
        "    jal zero, 0\n",
    ]
    .concat();
    let machine_code = assemble(&assembly, Arch::Rv64)
        .expect("fixed DUP1 RV64 assembly must assemble into base RV64 instructions");
    assert_eq!(machine_code.len(), 16 * 4, "DUP1 encoding changed width");

    GeneratedRv64 {
        assembly,
        machine_code,
        link_metadata: None,
    }
}

pub(crate) fn generate_symbolic_dup1(index: usize, continuation: String) -> SymbolicFragment {
    let entry = format!("guest_dup1_{index}");
    let underflow = format!("dup1_underflow_{index}");
    let overflow = format!("dup1_overflow_{index}");
    let mut assembly = String::new();
    writeln!(assembly, "{entry}:").unwrap();
    writeln!(
        assembly,
        "    bgeu s0, s1, {underflow}\n\
         \x20   lui t0, 8\n\
         \x20   add t0, s0, t0\n\
         \x20   bgeu s1, t0, {overflow}\n\
         \x20   ld t1, -32(s1)\n\
         \x20   ld t2, -24(s1)\n\
         \x20   ld t3, -16(s1)\n\
         \x20   ld t4, -8(s1)\n\
         \x20   sd t1, 0(s1)\n\
         \x20   sd t2, 8(s1)\n\
         \x20   sd t3, 16(s1)\n\
         \x20   sd t4, 24(s1)\n\
         \x20   addi s1, s1, 32\n\
         \x20   jal zero, {continuation}\n\
         {underflow}:\n\
         \x20   jal zero, {}\n\
         {overflow}:\n\
         \x20   jal zero, {}",
        PublisherRole::Underflow.label(),
        PublisherRole::Overflow.label()
    )
    .unwrap();

    let mut external_references = vec![
        ExternalDeclaration {
            site_word: 14,
            role: PublisherRole::Underflow,
            kind: Transfer::Jal { rd: 0 },
        },
        ExternalDeclaration {
            site_word: 15,
            role: PublisherRole::Overflow,
            kind: Transfer::Jal { rd: 0 },
        },
    ];
    let local_references = if continuation == PublisherRole::Normal.label() {
        external_references.push(ExternalDeclaration {
            site_word: 13,
            role: PublisherRole::Normal,
            kind: Transfer::Jal { rd: 0 },
        });
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
                site_word: 3,
                target_symbol: overflow,
                kind: Transfer::Branch {
                    funct3: 7,
                    rs1: 9,
                    rs2: 5,
                },
            },
        ]
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
                site_word: 3,
                target_symbol: overflow,
                kind: Transfer::Branch {
                    funct3: 7,
                    rs1: 9,
                    rs2: 5,
                },
            },
            LocalDeclaration {
                site_word: 13,
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
        Fragment::Dup1,
    )
}

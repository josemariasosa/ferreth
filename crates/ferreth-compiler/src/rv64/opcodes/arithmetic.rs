//! Shared symbolic stack wrapper for the pointer-based arithmetic leaves.

use crate::{
    compiler::symbolic::{ExternalDeclaration, LocalDeclaration, PublisherRole, SymbolicFragment},
    rv64::verify::{ArithmeticCore, Fragment, Transfer},
};

pub(super) fn generate_symbolic_arithmetic(
    index: usize,
    continuation: String,
    write_core: fn(&mut String),
    core: ArithmeticCore,
) -> SymbolicFragment {
    let entry = format!("guest_arithmetic_{index}");
    let underflow = format!("arith_underflow_{index}");
    let leaf = format!("arith_core_{index}");
    let update = format!("arith_update_{index}");
    let mut source = format!(
        "{entry}:\n\
         \x20   addi t0, s0, 64\n\
         \x20   bltu s1, t0, {underflow}\n\
         \x20   addi a0, s1, -32\n\
         \x20   addi a1, s1, -64\n\
         \x20   addi a2, s1, -64\n\
         \x20   jal ra, {leaf}\n\
         {update}:\n\
         \x20   addi s1, s1, -32\n\
         \x20   jal zero, {continuation}\n\
         {underflow}:\n\
         \x20   jal zero, {}\n\
         {leaf}:\n",
        PublisherRole::Underflow.label()
    );
    write_core(&mut source);

    let mut externals = vec![ExternalDeclaration {
        site_word: 8,
        role: PublisherRole::Underflow,
        kind: Transfer::Jal { rd: 0 },
    }];
    let mut locals = vec![
        LocalDeclaration {
            site_word: 1,
            target_symbol: underflow,
            kind: Transfer::Branch {
                funct3: 6,
                rs1: 9,
                rs2: 5,
            },
        },
        LocalDeclaration {
            site_word: 5,
            target_symbol: leaf,
            kind: Transfer::Jal { rd: 1 },
        },
        LocalDeclaration {
            site_word: 42,
            target_symbol: update,
            kind: Transfer::LeafReturn,
        },
    ];
    if continuation == PublisherRole::Normal.label() {
        externals.push(ExternalDeclaration {
            site_word: 7,
            role: PublisherRole::Normal,
            kind: Transfer::Jal { rd: 0 },
        });
    } else {
        locals.push(LocalDeclaration {
            site_word: 7,
            target_symbol: continuation,
            kind: Transfer::Jal { rd: 0 },
        });
    }
    SymbolicFragment::new(
        source,
        entry,
        externals,
        locals,
        Fragment::SymbolicArithmetic(core),
    )
}

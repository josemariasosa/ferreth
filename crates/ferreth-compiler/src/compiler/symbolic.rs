//! Single-image composition for generated symbolic RV64 fragments.

use std::collections::HashSet;

use asm_rs::{Arch, Assembler, AssemblyResult};

use super::{assembly_error, verification_error, CompileError, CompiledRv64Program, Publisher};
use crate::{
    decode_detailed,
    rv64::{
        memory::{validate_generated_code, ENTRY_PC, RV64_INSTRUCTION_SIZE},
        opcodes::push::generate_symbolic_push1,
        verify::{
            verify_declared_transfers, verify_final_image, DeclaredTransfer, Fragment,
            PublisherLayout, Transfer,
        },
    },
    DecodedInstruction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PublisherRole {
    Normal,
    Overflow,
}

impl PublisherRole {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal_exit",
            Self::Overflow => "push1_stack_overflow_exit",
        }
    }

    fn publisher(self) -> Publisher {
        match self {
            Self::Normal => Publisher::Normal,
            Self::Overflow => Publisher::Overflow,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalDeclaration {
    /// Word index relative to the owning fragment's entry symbol.
    pub(crate) site_word: u64,
    pub(crate) role: PublisherRole,
    pub(crate) kind: Transfer,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalDeclaration {
    /// Word index relative to the owning fragment's entry symbol.
    pub(crate) site_word: u64,
    pub(crate) target_symbol: String,
    pub(crate) kind: Transfer,
}

/// Generated source and its transfer contract. The entry symbol is the
/// fragment's ownership key; generators use its deterministic occurrence index
/// for all local symbols.
/// `fixed_layout` feeds only the independent Phase 2.1 completeness check.
#[derive(Clone)]
pub(crate) struct SymbolicFragment {
    source: String,
    entry_symbol: String,
    external_references: Vec<ExternalDeclaration>,
    local_transfers: Vec<LocalDeclaration>,
    fixed_layout: Fragment,
}

impl SymbolicFragment {
    pub(crate) fn new(
        source: String,
        entry_symbol: String,
        external_references: Vec<ExternalDeclaration>,
        local_transfers: Vec<LocalDeclaration>,
        fixed_layout: Fragment,
    ) -> Self {
        Self {
            source,
            entry_symbol,
            external_references,
            local_transfers,
            fixed_layout,
        }
    }
}

fn compile(bytecode: &[u8]) -> Result<CompiledRv64Program, CompileError> {
    let fragments = decode_detailed(bytecode)?
        .into_iter()
        .enumerate()
        .map(|(index, instruction)| match instruction {
            DecodedInstruction::Stop { .. } => Ok(stop_fragment()),
            DecodedInstruction::Push {
                value,
                opcode: 0x60,
                ..
            } => Ok(generate_symbolic_push1(value.to::<u8>(), index)),
            DecodedInstruction::Push { opcode, offset, .. } => {
                Err(CompileError::UnsupportedInstruction { opcode, offset })
            }
            DecodedInstruction::Add { offset } => Err(CompileError::UnsupportedInstruction {
                opcode: 0x01,
                offset,
            }),
            DecodedInstruction::Sub { offset } => Err(CompileError::UnsupportedInstruction {
                opcode: 0x03,
                offset,
            }),
            DecodedInstruction::Pop { offset } => Err(CompileError::UnsupportedInstruction {
                opcode: 0x50,
                offset,
            }),
            DecodedInstruction::Dup1 { offset } => Err(CompileError::UnsupportedInstruction {
                opcode: 0x80,
                offset,
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    compose(&fragments)
}

fn stop_fragment() -> SymbolicFragment {
    let entry = "guest_stop";
    let role = PublisherRole::Normal;
    SymbolicFragment::new(
        format!("{entry}:\n    jal zero, {}\n", role.label()),
        entry.to_owned(),
        vec![ExternalDeclaration {
            site_word: 0,
            role,
            kind: Transfer::Jal { rd: 0 },
        }],
        Vec::new(),
        Fragment::Stop,
    )
}

fn compose(fragments: &[SymbolicFragment]) -> Result<CompiledRv64Program, CompileError> {
    let mut entries = HashSet::with_capacity(fragments.len());
    for fragment in fragments {
        if !entries.insert(fragment.entry_symbol.as_str()) {
            return Err(verification_error(format!(
                "duplicate fragment entry symbol {}",
                fragment.entry_symbol
            )));
        }
    }

    let publishers = publisher_inventory(fragments);
    let source = emit(fragments, &publishers);
    let image = assemble_image(&source)?;
    verified_program(fragments, &publishers, source, image)
}

fn publisher_inventory(fragments: &[SymbolicFragment]) -> Vec<PublisherRole> {
    let mut roles = vec![PublisherRole::Normal];
    roles.extend(
        fragments
            .iter()
            .flat_map(|fragment| &fragment.external_references)
            .map(|declaration| declaration.role),
    );
    roles.sort_unstable();
    roles.dedup();
    roles
}

fn emit(fragments: &[SymbolicFragment], publishers: &[PublisherRole]) -> String {
    let mut source = String::new();
    for fragment in fragments {
        source.push_str(&fragment.source);
    }
    for role in publishers {
        source.push_str(&role.publisher().assembly());
    }
    source
}

fn assemble_image(source: &str) -> Result<AssemblyResult, CompileError> {
    let mut assembler = Assembler::new(Arch::Rv64);
    assembler
        .base_address(ENTRY_PC)
        .emit(source)
        .map_err(assembly_error)?;
    assembler.finish().map_err(assembly_error)
}

fn verified_program(
    fragments: &[SymbolicFragment],
    publishers: &[PublisherRole],
    assembly: String,
    image: AssemblyResult,
) -> Result<CompiledRv64Program, CompileError> {
    validate_generated_code(image.base_address(), image.len())?;
    let publisher_pcs = publishers
        .iter()
        .map(|role| {
            let label = role.label();
            image
                .label_address(label)
                .ok_or_else(|| verification_error(format!("missing publisher label {label}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let publisher_pc = |role: PublisherRole| {
        publishers
            .iter()
            .position(|candidate| *candidate == role)
            .map(|index| publisher_pcs[index])
            .ok_or_else(|| verification_error(format!("missing {role:?} publisher")))
    };

    let mut declarations = Vec::new();
    for fragment in fragments {
        let entry_pc = image.label_address(&fragment.entry_symbol).ok_or_else(|| {
            verification_error(format!(
                "missing fragment entry label {}",
                fragment.entry_symbol
            ))
        })?;
        for declaration in &fragment.local_transfers {
            let target_pc = image
                .label_address(&declaration.target_symbol)
                .ok_or_else(|| {
                    verification_error(format!(
                        "missing local target label {}",
                        declaration.target_symbol
                    ))
                })?;
            declarations.push(DeclaredTransfer {
                site_pc: transfer_site_pc(entry_pc, declaration.site_word)?,
                target_pc,
                kind: declaration.kind,
            });
        }
        for declaration in &fragment.external_references {
            declarations.push(DeclaredTransfer {
                site_pc: transfer_site_pc(entry_pc, declaration.site_word)?,
                target_pc: publisher_pc(declaration.role)?,
                kind: declaration.kind,
            });
        }
    }

    let normal_pc = publisher_pc(PublisherRole::Normal)?;
    verify_declared_transfers(image.bytes(), normal_pc, &declarations)
        .map_err(verification_error)?;
    verify_final_image(
        image.bytes(),
        fragments.iter().map(|fragment| fragment.fixed_layout),
        PublisherLayout::Symbolic,
        &publisher_pcs,
    )
    .map_err(verification_error)?;

    let end = ENTRY_PC
        .checked_add(image.len() as u64)
        .ok_or_else(|| verification_error("image end address overflow".to_owned()))?;
    let normal_index = publishers
        .iter()
        .position(|role| *role == PublisherRole::Normal)
        .expect("normal termination is always included");
    let normal_end = publisher_pcs.get(normal_index + 1).copied().unwrap_or(end);
    let exceptional_termination_instructions = publisher_pcs
        .get(normal_index + 1)
        .map_or(0, |overflow_pc| (end - overflow_pc) / RV64_INSTRUCTION_SIZE);
    Ok(CompiledRv64Program {
        assembly,
        machine_code: image.into_bytes(),
        normal_termination_instructions: (normal_end - publisher_pcs[normal_index])
            / RV64_INSTRUCTION_SIZE,
        exceptional_termination_instructions,
        termination_entry_pcs: publisher_pcs,
    })
}

fn transfer_site_pc(entry_pc: u64, site_word: u64) -> Result<u64, CompileError> {
    site_word
        .checked_mul(RV64_INSTRUCTION_SIZE)
        .and_then(|offset| entry_pc.checked_add(offset))
        .ok_or_else(|| verification_error("transfer site address overflow".to_owned()))
}

#[cfg(test)]
mod tests;

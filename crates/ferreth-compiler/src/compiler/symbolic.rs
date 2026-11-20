//! Single-image composition for PUSH1 and STOP.

use std::fmt::Write;

use asm_rs::{Arch, Assembler, AssemblyResult};

use super::{assembly_error, verification_error, CompileError, CompiledRv64Program, Publisher};
use crate::{
    decode_detailed,
    rv64::{
        memory::{validate_generated_code, ENTRY_PC, RV64_INSTRUCTION_SIZE},
        opcodes::push::emit_symbolic_push1,
        verify::{verify_final_image, Fragment, PublisherLayout},
    },
    DecodedInstruction,
};

const PUBLISHERS: [Publisher; 2] = [Publisher::Normal, Publisher::Overflow];

#[derive(Clone, Copy)]
enum SymbolicFragment {
    Push1(u8),
    Stop,
}

fn compile(bytecode: &[u8]) -> Result<CompiledRv64Program, CompileError> {
    let fragments = decode_detailed(bytecode)?
        .into_iter()
        .map(|instruction| {
            let (opcode, offset) = match instruction {
                DecodedInstruction::Stop { .. } => return Ok(SymbolicFragment::Stop),
                DecodedInstruction::Push {
                    value,
                    opcode: 0x60,
                    ..
                } => return Ok(SymbolicFragment::Push1(value.to::<u8>())),
                DecodedInstruction::Push { opcode, offset, .. } => (opcode, offset),
                DecodedInstruction::Add { offset } => (0x01, offset),
                DecodedInstruction::Sub { offset } => (0x03, offset),
                DecodedInstruction::Pop { offset } => (0x50, offset),
                DecodedInstruction::Dup1 { offset } => (0x80, offset),
            };
            Err(CompileError::UnsupportedInstruction { opcode, offset })
        })
        .collect::<Result<Vec<_>, _>>()?;
    compose(&fragments)
}

fn compose(fragments: &[SymbolicFragment]) -> Result<CompiledRv64Program, CompileError> {
    let source = emit(fragments);
    let image = assemble_image(&source)?;
    verified_program(fragments, source, image)
}

fn emit(fragments: &[SymbolicFragment]) -> String {
    let mut source = String::new();
    for (index, fragment) in fragments.iter().enumerate() {
        match *fragment {
            SymbolicFragment::Push1(immediate) => {
                emit_symbolic_push1(&mut source, immediate, index, Publisher::Overflow.label());
            }
            SymbolicFragment::Stop => {
                let normal = Publisher::Normal.label();
                writeln!(source, "guest_stop:\n    jal zero, {normal}").unwrap();
            }
        }
    }
    for publisher in PUBLISHERS {
        source.push_str(&publisher.assembly());
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
    assembly: String,
    image: AssemblyResult,
) -> Result<CompiledRv64Program, CompileError> {
    validate_generated_code(image.base_address(), image.len())?;
    let termination_entry_pcs = PUBLISHERS
        .into_iter()
        .map(|publisher| {
            let label = publisher.label();
            image
                .label_address(label)
                .ok_or_else(|| verification_error(format!("missing publisher label {label}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Each fragment kind declares its complete transfer sites, publisher
    // roles and local anchors in the same fixed layout checked on legacy.
    let expected = fragments.iter().map(|fragment| match fragment {
        SymbolicFragment::Push1(_) => Fragment::Push1,
        SymbolicFragment::Stop => Fragment::Stop,
    });
    verify_final_image(
        image.bytes(),
        expected,
        PublisherLayout::PushOnly,
        &termination_entry_pcs,
    )
    .map_err(verification_error)?;

    let end = ENTRY_PC + image.len() as u64;
    Ok(CompiledRv64Program {
        assembly,
        machine_code: image.into_bytes(),
        normal_termination_instructions: (termination_entry_pcs[1] - termination_entry_pcs[0])
            / RV64_INSTRUCTION_SIZE,
        exceptional_termination_instructions: (end - termination_entry_pcs[1])
            / RV64_INSTRUCTION_SIZE,
        termination_entry_pcs,
    })
}

#[cfg(test)]
mod tests;

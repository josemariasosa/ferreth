//! Minimal whole-program composition for EVM `STOP`, `PUSH1`, `ADD`, `SUB`, `POP`, and `DUP1` operations.
//!
//! This is deliberately not an EVM decoder or a general compiler.  It links
//! the reusable PUSH1 guest blocks into one executable and supplies the shared
//! terminal paths required by the initial guest ABI.

use std::fmt;

#[cfg(test)]
use asm_rs::{assemble, Arch};

use crate::rv64::memory::{LayoutError, ENTRY_PC, EXIT_PC, RV64_INSTRUCTION_SIZE};
#[cfg(test)]
use crate::rv64::{
    codegen::{GeneratedRv64, GeneratedRv64LinkMetadata},
    memory::validate_generated_code,
    opcodes::{
        add::generate_guest_add, dup::generate_guest_dup1, pop::generate_guest_pop,
        push::generate_guest_push1, sub::generate_guest_sub,
    },
    verify::{verify_final_image, Fragment, PublisherLayout},
};
use crate::DecodeError;
#[cfg(test)]
use crate::{decode_detailed, DecodedInstruction};

mod publisher;
pub(crate) mod symbolic;

use publisher::Publisher;

/// Provisional guest exception code for an EVM stack-overflow push.
pub const PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE: u64 = 1;
/// Provisional guest exception code for an EVM binary-operation underflow.
pub const PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE: u64 = 2;

/// A linked RV64 program made from the supported guest operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRv64Program {
    assembly: String,
    machine_code: Vec<u8>,
    normal_termination_instructions: u64,
    exceptional_termination_instructions: u64,
    termination_entry_pcs: Vec<u64>,
}

impl CompiledRv64Program {
    pub fn assembly(&self) -> &str {
        &self.assembly
    }

    pub fn machine_code(&self) -> &[u8] {
        &self.machine_code
    }

    pub const fn entry_pc(&self) -> u64 {
        ENTRY_PC
    }

    pub const fn exit_pc(&self) -> u64 {
        EXIT_PC
    }

    /// Conservative instruction limit: every emitted instruction at most once.
    pub fn instruction_limit(&self) -> u64 {
        self.machine_code.len() as u64 / RV64_INSTRUCTION_SIZE
    }

    pub const fn normal_termination_instructions(&self) -> u64 {
        self.normal_termination_instructions
    }

    /// Number of resident exceptional publisher words in the compiled image.
    ///
    /// Images contain 0 words with neither publisher, 8 with Underflow or
    /// Overflow alone, and 16 with both. During exceptional execution only
    /// the selected publisher runs, for eight dynamic instructions.
    pub const fn exceptional_termination_instructions(&self) -> u64 {
        self.exceptional_termination_instructions
    }

    /// RV64 PCs at which shared normal or exceptional publishers begin.
    pub fn termination_entry_pcs(&self) -> &[u64] {
        &self.termination_entry_pcs
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    Decode(DecodeError),
    UnsupportedInstruction { opcode: u8, offset: usize },
    Layout(LayoutError),
    Link { description: String },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "could not decode EVM bytecode: {error}"),
            Self::UnsupportedInstruction { offset, opcode } => write!(
                f,
                "EVM opcode 0x{opcode:02x} at byte offset {offset} is decoded but not supported by the RV64 compiler"
            ),
            Self::Layout(error) => write!(f, "invalid compiled guest layout: {error}"),
            Self::Link { description } => write!(f, "could not link guest program: {description}"),
        }
    }
}

impl std::error::Error for CompileError {}

impl From<LayoutError> for CompileError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<DecodeError> for CompileError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// Decodes and compiles the supported EVM bytecode subset into one RV64 executable.
///
/// Reaching the end of bytecode falls through to the compatibility normal
/// epilogue. An explicit `STOP` emits a direct transfer to that same epilogue.
pub fn compile_evm_bytecode(bytecode: &[u8]) -> Result<CompiledRv64Program, CompileError> {
    symbolic::compile(bytecode)
}

#[cfg(test)]
pub(crate) fn compile_evm_bytecode_legacy(
    bytecode: &[u8],
) -> Result<CompiledRv64Program, CompileError> {
    let instructions = decode_detailed(bytecode)?;
    let mut operations = Vec::with_capacity(instructions.len());
    let mut has_explicit_stop = false;
    for instruction in instructions {
        match instruction {
            DecodedInstruction::Stop { .. } => {
                has_explicit_stop = true;
                break;
            }
            DecodedInstruction::Push {
                value,
                opcode: 0x60,
                ..
            } => operations.push(CompiledOperation::Push1(value.to::<u8>())),
            DecodedInstruction::Push { opcode, offset, .. } => {
                return Err(CompileError::UnsupportedInstruction { opcode, offset });
            }
            DecodedInstruction::Add { .. } => operations.push(CompiledOperation::Add),
            DecodedInstruction::Sub { .. } => operations.push(CompiledOperation::Sub),
            DecodedInstruction::Pop { .. } => operations.push(CompiledOperation::Pop),
            DecodedInstruction::Dup1 { .. } => operations.push(CompiledOperation::Dup1),
        }
    }
    // Keep the established PUSH1-only linker (and therefore its exact output)
    // for existing callers. Mixed programs use the small linker below.
    if !has_explicit_stop
        && operations
            .iter()
            .all(|operation| matches!(operation, CompiledOperation::Push1(_)))
    {
        return compile_push1_program_legacy(
            &operations
                .iter()
                .map(|operation| match operation {
                    CompiledOperation::Push1(immediate) => *immediate,
                    CompiledOperation::Add
                    | CompiledOperation::Sub
                    | CompiledOperation::Pop
                    | CompiledOperation::Dup1 => unreachable!("checked above"),
                })
                .collect::<Vec<_>>(),
        );
    }
    compile_supported_program_legacy(&operations, has_explicit_stop)
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
enum CompiledOperation {
    Push1(u8),
    Add,
    Sub,
    Pop,
    Dup1,
}

/// Links a mixed sequence of supported opcode bodies. Reaching the end falls
/// through to the compatibility normal epilogue; an explicit STOP jumps there.
#[cfg(test)]
fn compile_supported_program_legacy(
    operations: &[CompiledOperation],
    has_explicit_stop: bool,
) -> Result<CompiledRv64Program, CompileError> {
    let generated = operations
        .iter()
        .map(|operation| match operation {
            CompiledOperation::Push1(immediate) => Ok(GeneratedOperation::Push1 {
                code: generate_guest_push1(*immediate),
            }),
            CompiledOperation::Add => {
                let code = generate_guest_add();
                let links = code.link_metadata().ok_or_else(|| CompileError::Link {
                    description: "ADD generator did not provide link metadata".to_owned(),
                })?;
                if links.normal_continuation_offset() != code.machine_code().len()
                    || instruction_at(code.machine_code(), links.core_call_offset())? & 0x0000_0fff
                        != 0x0000_00ef
                    || instruction_at(code.machine_code(), links.exceptional_trampoline_offset())?
                        != 0x0000_006f
                {
                    return Err(CompileError::Link {
                        description: "ADD generator link metadata no longer describes its wrapper"
                            .to_owned(),
                    });
                }
                Ok(GeneratedOperation::Add { code, links })
            }
            CompiledOperation::Sub => {
                let code = generate_guest_sub();
                let links = code.link_metadata().ok_or_else(|| CompileError::Link {
                    description: "SUB generator did not provide link metadata".to_owned(),
                })?;
                if links.normal_continuation_offset() != code.machine_code().len()
                    || instruction_at(code.machine_code(), links.core_call_offset())? & 0x0000_0fff
                        != 0x0000_00ef
                    || instruction_at(code.machine_code(), links.exceptional_trampoline_offset())?
                        != 0x0000_006f
                {
                    return Err(CompileError::Link {
                        description: "SUB generator link metadata no longer describes its wrapper"
                            .to_owned(),
                    });
                }
                Ok(GeneratedOperation::Sub { code, links })
            }
            CompiledOperation::Pop => {
                let code = generate_guest_pop();
                let trampoline_offset = code.machine_code().len() - RV64_INSTRUCTION_SIZE as usize;
                if instruction_at(code.machine_code(), trampoline_offset)? != 0x0000_006f {
                    return Err(CompileError::Link {
                        description:
                            "POP generator no longer has its documented underflow trampoline"
                                .to_owned(),
                    });
                }
                Ok(GeneratedOperation::Pop {
                    code,
                    trampoline_offset,
                })
            }
            CompiledOperation::Dup1 => {
                let code = generate_guest_dup1();
                let underflow_trampoline_offset =
                    code.machine_code().len() - 2 * RV64_INSTRUCTION_SIZE as usize;
                let overflow_trampoline_offset =
                    code.machine_code().len() - RV64_INSTRUCTION_SIZE as usize;
                if instruction_at(code.machine_code(), underflow_trampoline_offset)? != 0x0000_006f
                    || instruction_at(code.machine_code(), overflow_trampoline_offset)?
                        != 0x0000_006f
                {
                    return Err(CompileError::Link {
                        description:
                            "DUP1 generator no longer has its underflow and overflow trampolines"
                                .to_owned(),
                    });
                }
                Ok(GeneratedOperation::Dup1 {
                    code,
                    underflow_trampoline_offset,
                    overflow_trampoline_offset,
                })
            }
        })
        .collect::<Result<Vec<_>, CompileError>>()?;

    let normal_assembly = Publisher::Normal.assembly();
    let overflow_assembly = Publisher::Overflow.assembly();
    let underflow_assembly = Publisher::Underflow.assembly();
    let normal_code = assemble(&normal_assembly, Arch::Rv64).map_err(assembly_error)?;
    let overflow_code = assemble(&overflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    let underflow_code = assemble(&underflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    let opcode_len = generated.iter().map(GeneratedOperation::len).sum::<usize>();
    // STOP is one `jal zero, 4`, which skips directly to the shared normal
    // publisher. It does not alter the EVM stack registers or native `sp`.
    let stop_len = usize::from(has_explicit_stop) * RV64_INSTRUCTION_SIZE as usize;
    let normal_start =
        ENTRY_PC + u64::try_from(opcode_len + stop_len).expect("opcode length fits u64");
    let overflow_start =
        normal_start + u64::try_from(normal_code.len()).expect("code length fits u64");
    let underflow_start =
        overflow_start + u64::try_from(overflow_code.len()).expect("code length fits u64");

    let mut assembly = String::new();
    let mut machine_code = Vec::with_capacity(
        opcode_len + stop_len + normal_code.len() + overflow_code.len() + underflow_code.len(),
    );
    for (index, operation) in generated.iter().enumerate() {
        let operation_start =
            ENTRY_PC + u64::try_from(machine_code.len()).expect("offset fits u64");
        match operation {
            GeneratedOperation::Push1 { code, .. } => {
                let body = code
                    .assembly()
                    .strip_prefix("guest_push1:\n")
                    .and_then(|source| source.strip_suffix("    jal zero, 0\n"))
                    .ok_or_else(|| CompileError::Link {
                        description: "PUSH1 generator no longer has its documented trampoline"
                            .to_owned(),
                    })?;
                let trampoline_pc = operation_start
                    + u64::try_from(code.overflow_trampoline_offset()).expect("offset fits u64");
                let offset = relative_jal_offset(trampoline_pc, overflow_start)?;
                assembly.push_str(&format!(
                    "guest_push1_{index}:\n{body}    jal zero, {offset}\n"
                ));
                let mut bytes = code.machine_code().to_vec();
                let trampoline = encode_jal_zero(offset)?;
                let trampoline_offset = code.overflow_trampoline_offset();
                bytes[trampoline_offset..trampoline_offset + 4]
                    .copy_from_slice(&trampoline.to_le_bytes());
                machine_code.extend_from_slice(&bytes);
            }
            GeneratedOperation::Add { code, links } => {
                let trampoline_pc = operation_start
                    + u64::try_from(links.exceptional_trampoline_offset())
                        .expect("offset fits u64");
                let offset = relative_jal_offset(trampoline_pc, underflow_start)?;
                assembly.push_str(&linked_add_assembly(code.assembly(), index, offset)?);
                let mut bytes = code.machine_code().to_vec();
                let trampoline = encode_jal_zero(offset)?;
                let trampoline_offset = links.exceptional_trampoline_offset();
                bytes[trampoline_offset..trampoline_offset + 4]
                    .copy_from_slice(&trampoline.to_le_bytes());
                machine_code.extend_from_slice(&bytes);
            }
            GeneratedOperation::Sub { code, links } => {
                let trampoline_pc = operation_start
                    + u64::try_from(links.exceptional_trampoline_offset())
                        .expect("offset fits u64");
                let offset = relative_jal_offset(trampoline_pc, underflow_start)?;
                assembly.push_str(&linked_sub_assembly(code.assembly(), index, offset)?);
                let mut bytes = code.machine_code().to_vec();
                let trampoline = encode_jal_zero(offset)?;
                let trampoline_offset = links.exceptional_trampoline_offset();
                bytes[trampoline_offset..trampoline_offset + 4]
                    .copy_from_slice(&trampoline.to_le_bytes());
                machine_code.extend_from_slice(&bytes);
            }
            GeneratedOperation::Pop {
                code,
                trampoline_offset,
            } => {
                let trampoline_pc =
                    operation_start + u64::try_from(*trampoline_offset).expect("offset fits u64");
                let offset = relative_jal_offset(trampoline_pc, underflow_start)?;
                assembly.push_str(&linked_pop_assembly(code.assembly(), index, offset)?);
                let mut bytes = code.machine_code().to_vec();
                bytes[*trampoline_offset..*trampoline_offset + 4]
                    .copy_from_slice(&encode_jal_zero(offset)?.to_le_bytes());
                machine_code.extend_from_slice(&bytes);
            }
            GeneratedOperation::Dup1 {
                code,
                underflow_trampoline_offset,
                overflow_trampoline_offset,
            } => {
                let underflow_pc = operation_start
                    + u64::try_from(*underflow_trampoline_offset).expect("offset fits u64");
                let overflow_pc = operation_start
                    + u64::try_from(*overflow_trampoline_offset).expect("offset fits u64");
                let underflow_offset = relative_jal_offset(underflow_pc, underflow_start)?;
                let overflow_offset = relative_jal_offset(overflow_pc, overflow_start)?;
                assembly.push_str(&linked_dup1_assembly(
                    code.assembly(),
                    index,
                    underflow_offset,
                    overflow_offset,
                )?);
                let mut bytes = code.machine_code().to_vec();
                bytes[*underflow_trampoline_offset..*underflow_trampoline_offset + 4]
                    .copy_from_slice(&encode_jal_zero(underflow_offset)?.to_le_bytes());
                bytes[*overflow_trampoline_offset..*overflow_trampoline_offset + 4]
                    .copy_from_slice(&encode_jal_zero(overflow_offset)?.to_le_bytes());
                machine_code.extend_from_slice(&bytes);
            }
        }
    }
    if has_explicit_stop {
        assembly.push_str("evm_stop:\n    jal zero, 4\n");
        machine_code.extend_from_slice(&encode_jal_zero(4)?.to_le_bytes());
    }
    assembly.push_str(&normal_assembly);
    assembly.push_str(&overflow_assembly);
    assembly.push_str(&underflow_assembly);
    machine_code.extend_from_slice(&normal_code);
    machine_code.extend_from_slice(&overflow_code);
    machine_code.extend_from_slice(&underflow_code);

    if assemble(&assembly, Arch::Rv64).map_err(assembly_error)? != machine_code {
        return Err(CompileError::Link {
            description: "linked display assembly does not reproduce loaded machine code"
                .to_owned(),
        });
    }
    validate_generated_code(ENTRY_PC, machine_code.len())?;
    let termination_entry_pcs = vec![normal_start, overflow_start, underflow_start];
    let fragments = operations
        .iter()
        .map(|operation| match operation {
            CompiledOperation::Push1(_) => Fragment::Push1,
            CompiledOperation::Add | CompiledOperation::Sub => Fragment::Arithmetic,
            CompiledOperation::Pop => Fragment::Pop,
            CompiledOperation::Dup1 => Fragment::Dup1,
        })
        .chain(has_explicit_stop.then_some(Fragment::Stop));
    verify_final_image(
        &machine_code,
        fragments,
        PublisherLayout::Mixed,
        &termination_entry_pcs,
    )
    .map_err(verification_error)?;
    Ok(CompiledRv64Program {
        assembly,
        machine_code,
        normal_termination_instructions: instruction_count(&normal_code)?,
        // Both currently supported guest-exception publishers are eight instructions.
        exceptional_termination_instructions: instruction_count(&overflow_code)?,
        termination_entry_pcs,
    })
}

#[cfg(test)]
enum GeneratedOperation {
    Push1 {
        code: GeneratedRv64,
    },
    Add {
        code: GeneratedRv64,
        links: GeneratedRv64LinkMetadata,
    },
    Sub {
        code: GeneratedRv64,
        links: GeneratedRv64LinkMetadata,
    },
    Pop {
        code: GeneratedRv64,
        trampoline_offset: usize,
    },
    Dup1 {
        code: GeneratedRv64,
        underflow_trampoline_offset: usize,
        overflow_trampoline_offset: usize,
    },
}

#[cfg(test)]
impl GeneratedOperation {
    fn len(&self) -> usize {
        match self {
            Self::Push1 { code, .. }
            | Self::Add { code, .. }
            | Self::Sub { code, .. }
            | Self::Pop { code, .. }
            | Self::Dup1 { code, .. } => code.machine_code().len(),
        }
    }
}

#[cfg(test)]
fn linked_dup1_assembly(
    source: &str,
    index: usize,
    underflow_offset: i64,
    overflow_offset: i64,
) -> Result<String, CompileError> {
    let source = source.replacen("guest_dup1:\n", &format!("guest_dup1_{index}:\n"), 1);
    let source = source.replacen(
        "guest_dup1_underflow",
        &format!("guest_dup1_underflow_{index}"),
        2,
    );
    let source = source.replacen(
        "guest_dup1_overflow",
        &format!("guest_dup1_overflow_{index}"),
        2,
    );
    let source = source.replacen(
        &format!("guest_dup1_underflow_{index}:\n    jal zero, 0"),
        &format!("guest_dup1_underflow_{index}:\n    jal zero, {underflow_offset}"),
        1,
    );
    let source = source.replacen(
        &format!("guest_dup1_overflow_{index}:\n    jal zero, 0"),
        &format!("guest_dup1_overflow_{index}:\n    jal zero, {overflow_offset}"),
        1,
    );
    if source.contains("guest_dup1_underflow:\n")
        || source.contains("guest_dup1_overflow:\n")
        || source.contains("jal zero, 0")
    {
        return Err(CompileError::Link {
            description: "DUP1 assembly no longer exposes both patchable trampolines".to_owned(),
        });
    }
    Ok(source)
}

#[cfg(test)]
fn linked_pop_assembly(
    source: &str,
    index: usize,
    underflow_offset: i64,
) -> Result<String, CompileError> {
    let source = source.replacen("guest_pop:\n", &format!("guest_pop_{index}:\n"), 1);
    let source = source.replacen(
        "bgeu s0, s1, guest_pop_underflow",
        &format!("bgeu s0, s1, guest_pop_underflow_{index}"),
        1,
    );
    let source = source.replacen(
        "guest_pop_underflow:\n    jal zero, 0",
        &format!("guest_pop_underflow_{index}:\n    jal zero, {underflow_offset}"),
        1,
    );
    if source.contains("guest_pop_underflow:\n") {
        return Err(CompileError::Link {
            description: "POP assembly no longer exposes a patchable underflow label".to_owned(),
        });
    }
    Ok(source)
}

#[cfg(test)]
fn linked_add_assembly(
    source: &str,
    index: usize,
    underflow_offset: i64,
) -> Result<String, CompileError> {
    let source = source.replacen("guest_add:\n", &format!("guest_add_{index}:\n"), 1);
    let source = source.replacen("jal ra, evm_add", &format!("jal ra, evm_add_{index}"), 1);
    let source = source.replacen(
        "bltu s1, t0, guest_add_underflow",
        &format!("bltu s1, t0, guest_add_underflow_{index}"),
        1,
    );
    let source = source.replacen(
        "guest_add_underflow:\n    jal zero, 0",
        &format!("guest_add_underflow_{index}:\n    jal zero, {underflow_offset}"),
        1,
    );
    let source = source.replacen("evm_add:\n", &format!("evm_add_{index}:\n"), 1);
    if source.contains("guest_add_underflow:\n") || source.contains("evm_add:\n") {
        return Err(CompileError::Link {
            description: "ADD assembly no longer exposes patchable local labels".to_owned(),
        });
    }
    Ok(source)
}

#[cfg(test)]
fn linked_sub_assembly(
    source: &str,
    index: usize,
    underflow_offset: i64,
) -> Result<String, CompileError> {
    let source = source.replacen("guest_sub:\n", &format!("guest_sub_{index}:\n"), 1);
    let source = source.replacen("jal ra, evm_sub", &format!("jal ra, evm_sub_{index}"), 1);
    let source = source.replacen(
        "bltu s1, t0, guest_sub_underflow",
        &format!("bltu s1, t0, guest_sub_underflow_{index}"),
        1,
    );
    let source = source.replacen(
        "guest_sub_underflow:\n    jal zero, 0",
        &format!("guest_sub_underflow_{index}:\n    jal zero, {underflow_offset}"),
        1,
    );
    let source = source.replacen("evm_sub:\n", &format!("evm_sub_{index}:\n"), 1);
    if source.contains("guest_sub_underflow:\n") || source.contains("evm_sub:\n") {
        return Err(CompileError::Link {
            description: "SUB assembly no longer exposes patchable local labels".to_owned(),
        });
    }
    Ok(source)
}

#[cfg(test)]
pub(crate) fn compile_push1_program_legacy(
    immediates: &[u8],
) -> Result<CompiledRv64Program, CompileError> {
    let generated = immediates
        .iter()
        .copied()
        .map(generate_guest_push1)
        .collect::<Vec<_>>();
    let normal_assembly = Publisher::Normal.assembly();
    let overflow_assembly = Publisher::Overflow.assembly();
    let normal_code = assemble(&normal_assembly, Arch::Rv64).map_err(assembly_error)?;
    let overflow_code = assemble(&overflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    let opcode_len = generated
        .iter()
        .map(|code| code.machine_code().len())
        .sum::<usize>();
    let normal_start = ENTRY_PC + u64::try_from(opcode_len).expect("opcode length fits u64");
    let overflow_start =
        normal_start + u64::try_from(normal_code.len()).expect("code length fits u64");

    let mut assembly = String::new();
    let mut machine_code = Vec::with_capacity(opcode_len + normal_code.len() + overflow_code.len());
    for (index, code) in generated.iter().enumerate() {
        let opcode_assembly = code
            .assembly()
            .strip_prefix("guest_push1:\n")
            .and_then(|source| source.strip_suffix("    jal zero, 0\n"))
            .ok_or_else(|| CompileError::Link {
                description: "PUSH1 generator no longer has its documented labels and trampoline"
                    .to_owned(),
            })?;
        let trampoline_pc = ENTRY_PC
            + u64::try_from(machine_code.len() + code.overflow_trampoline_offset())
                .expect("offset fits u64");
        let trampoline_offset = relative_jal_offset(trampoline_pc, overflow_start)?;
        assembly.push_str(&format!(
            "guest_push1_{index}:\n{opcode_assembly}    jal zero, {trampoline_offset}\n"
        ));

        let mut bytes = code.machine_code().to_vec();
        let trampoline = encode_jal_zero(trampoline_offset)?;
        let offset = code.overflow_trampoline_offset();
        bytes[offset..offset + 4].copy_from_slice(&trampoline.to_le_bytes());
        machine_code.extend_from_slice(&bytes);
    }
    assembly.push_str(&normal_assembly);
    assembly.push_str(&overflow_assembly);
    machine_code.extend_from_slice(&normal_code);
    machine_code.extend_from_slice(&overflow_code);

    if assemble(&assembly, Arch::Rv64).map_err(assembly_error)? != machine_code {
        return Err(CompileError::Link {
            description: "linked display assembly does not reproduce loaded machine code"
                .to_owned(),
        });
    }
    validate_generated_code(ENTRY_PC, machine_code.len())?;
    let termination_entry_pcs = vec![normal_start, overflow_start];
    verify_final_image(
        &machine_code,
        std::iter::repeat_n(Fragment::Push1, immediates.len()),
        PublisherLayout::PushOnly,
        &termination_entry_pcs,
    )
    .map_err(verification_error)?;
    Ok(CompiledRv64Program {
        assembly,
        machine_code,
        normal_termination_instructions: instruction_count(&normal_code)?,
        exceptional_termination_instructions: instruction_count(&overflow_code)?,
        termination_entry_pcs,
    })
}

fn assembly_error(error: impl fmt::Debug) -> CompileError {
    CompileError::Link {
        description: format!("RV64 assembly failed: {error:?}"),
    }
}

fn verification_error(error: String) -> CompileError {
    CompileError::Link {
        description: format!("RV64 final-image verification failed: {error}"),
    }
}

#[cfg(test)]
fn instruction_count(machine_code: &[u8]) -> Result<u64, CompileError> {
    if !machine_code
        .len()
        .is_multiple_of(RV64_INSTRUCTION_SIZE as usize)
    {
        return Err(CompileError::Link {
            description: "linked code is not a sequence of 32-bit instructions".to_owned(),
        });
    }
    Ok((machine_code.len() / RV64_INSTRUCTION_SIZE as usize) as u64)
}

#[cfg(test)]
fn instruction_at(machine_code: &[u8], offset: usize) -> Result<u32, CompileError> {
    machine_code
        .get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| CompileError::Link {
            description: format!("arithmetic link offset {offset} is outside generated code"),
        })
}

#[cfg(test)]
fn relative_jal_offset(from_pc: u64, target_pc: u64) -> Result<i64, CompileError> {
    let offset = i64::try_from(i128::from(target_pc) - i128::from(from_pc)).map_err(|_| {
        CompileError::Link {
            description: "JAL offset does not fit an i64".to_owned(),
        }
    })?;
    encode_jal_zero(offset)?;
    Ok(offset)
}

#[cfg(test)]
fn encode_jal_zero(offset: i64) -> Result<u32, CompileError> {
    const MIN_JAL_OFFSET: i64 = -(1 << 20);
    const MAX_JAL_OFFSET: i64 = (1 << 20) - 2;
    if offset & 1 != 0 || !(MIN_JAL_OFFSET..=MAX_JAL_OFFSET).contains(&offset) {
        return Err(CompileError::Link {
            description: format!("JAL offset {offset} is not representable"),
        });
    }
    let immediate = offset as i32 as u32;
    Ok(((immediate & 0x7fe) << 20) | ((immediate & 0x800) << 9) | (immediate & 0xff000) | 0x6f)
}

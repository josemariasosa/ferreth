//! Minimal whole-program composition for EVM `PUSH1`, `ADD`, and `SUB` operations.
//!
//! This is deliberately not an EVM decoder or a general compiler.  It links
//! the reusable PUSH1 guest blocks into one executable and supplies the shared
//! terminal paths required by the initial guest ABI.

use std::fmt;

use asm_rs::{assemble, Arch};

use crate::rv64::{
    add::generate_guest_add,
    memory::{validate_generated_code, LayoutError, ENTRY_PC, EXIT_PC, RV64_INSTRUCTION_SIZE},
    push::generate_guest_push1,
    sub::generate_guest_sub,
};
use crate::{decode_detailed, DecodeError, DecodedInstruction};

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

    pub const fn exceptional_termination_instructions(&self) -> u64 {
        self.exceptional_termination_instructions
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
/// Reaching the end of the bytecode currently enters the temporary normal
/// termination epilogue; this is not complete EVM `STOP` semantics.
pub fn compile_evm_bytecode(bytecode: &[u8]) -> Result<CompiledRv64Program, CompileError> {
    let instructions = decode_detailed(bytecode)?;
    let mut operations = Vec::with_capacity(instructions.len());
    for instruction in instructions {
        match instruction {
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
        }
    }
    // Keep the established PUSH1-only linker (and therefore its exact output)
    // for existing callers. Mixed programs use the small linker below.
    if operations
        .iter()
        .all(|operation| matches!(operation, CompiledOperation::Push1(_)))
    {
        return compile_push1_program(
            &operations
                .iter()
                .map(|operation| match operation {
                    CompiledOperation::Push1(immediate) => *immediate,
                    CompiledOperation::Add | CompiledOperation::Sub => {
                        unreachable!("checked above")
                    }
                })
                .collect::<Vec<_>>(),
        );
    }
    compile_supported_program(&operations)
}

#[derive(Debug, Clone, Copy)]
enum CompiledOperation {
    Push1(u8),
    Add,
    Sub,
}

/// Links a mixed sequence of supported opcode bodies. Reaching the end still
/// uses the temporary normal epilogue rather than production EVM STOP.
fn compile_supported_program(
    operations: &[CompiledOperation],
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
        })
        .collect::<Result<Vec<_>, CompileError>>()?;

    let normal_assembly = normal_exit_assembly();
    let overflow_assembly = overflow_exit_assembly();
    let underflow_assembly = underflow_exit_assembly();
    let normal_code = assemble(&normal_assembly, Arch::Rv64).map_err(assembly_error)?;
    let overflow_code = assemble(&overflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    let underflow_code = assemble(&underflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    let opcode_len = generated.iter().map(GeneratedOperation::len).sum::<usize>();
    let normal_start = ENTRY_PC + u64::try_from(opcode_len).expect("opcode length fits u64");
    let overflow_start =
        normal_start + u64::try_from(normal_code.len()).expect("code length fits u64");
    let underflow_start =
        overflow_start + u64::try_from(overflow_code.len()).expect("code length fits u64");

    let mut assembly = String::new();
    let mut machine_code = Vec::with_capacity(
        opcode_len + normal_code.len() + overflow_code.len() + underflow_code.len(),
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
        }
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
    Ok(CompiledRv64Program {
        assembly,
        machine_code,
        normal_termination_instructions: instruction_count(&normal_code)?,
        // Both currently supported guest-exception publishers are eight instructions.
        exceptional_termination_instructions: instruction_count(&overflow_code)?,
    })
}

enum GeneratedOperation {
    Push1 {
        code: crate::rv64::push::GeneratedRv64,
    },
    Add {
        code: crate::rv64::push::GeneratedRv64,
        links: crate::rv64::push::GeneratedRv64LinkMetadata,
    },
    Sub {
        code: crate::rv64::push::GeneratedRv64,
        links: crate::rv64::push::GeneratedRv64LinkMetadata,
    },
}

impl GeneratedOperation {
    fn len(&self) -> usize {
        match self {
            Self::Push1 { code, .. } | Self::Add { code, .. } | Self::Sub { code, .. } => {
                code.machine_code().len()
            }
        }
    }
}

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

/// Compiles a sequence of `PUSH1` immediates into one RV64 executable.
///
/// Each opcode body comes directly from [`generate_guest_push1`]. Successful
/// bodies skip only their own trampoline and fall through to the next body;
/// every trampoline is patched to the single shared exceptional exit.
pub fn compile_push1_program(immediates: &[u8]) -> Result<CompiledRv64Program, CompileError> {
    let generated = immediates
        .iter()
        .copied()
        .map(generate_guest_push1)
        .collect::<Vec<_>>();
    let normal_assembly = normal_exit_assembly();
    let overflow_assembly = overflow_exit_assembly();
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
    Ok(CompiledRv64Program {
        assembly,
        machine_code,
        normal_termination_instructions: instruction_count(&normal_code)?,
        exceptional_termination_instructions: instruction_count(&overflow_code)?,
    })
}

fn normal_exit_assembly() -> String {
    "normal_exit:\n\
        \x20   addi t1, s0, -2048\n\
        \x20   addi t1, t1, -2048\n\
        \x20   sd s1, 16(t1)\n\
        \x20   sd zero, 8(t1)\n\
        \x20   addi t0, zero, 1\n\
        \x20   sd t0, 0(t1)\n\
        \x20   jalr zero, -4(t1)\n"
        .to_owned()
}

fn overflow_exit_assembly() -> String {
    format!(
        "push1_stack_overflow_exit:\n\
         \x20   addi t1, s0, -2048\n\
         \x20   addi t1, t1, -2048\n\
         \x20   sd s1, 16(t1)\n\
         \x20   addi t0, zero, {PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE}\n\
         \x20   sd t0, 8(t1)\n\
         \x20   addi t0, zero, 2\n\
         \x20   sd t0, 0(t1)\n\
         \x20   jalr zero, -4(t1)\n"
    )
}

fn underflow_exit_assembly() -> String {
    format!(
        "binary_stack_underflow_exit:\n\
         \x20   addi t1, s0, -2048\n\
         \x20   addi t1, t1, -2048\n\
         \x20   sd s1, 16(t1)\n\
         \x20   addi t0, zero, {PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE}\n\
         \x20   sd t0, 8(t1)\n\
         \x20   addi t0, zero, 2\n\
         \x20   sd t0, 0(t1)\n\
         \x20   jalr zero, -4(t1)\n"
    )
}

fn assembly_error(error: impl fmt::Debug) -> CompileError {
    CompileError::Link {
        description: format!("RV64 assembly failed: {error:?}"),
    }
}

fn instruction_count(machine_code: &[u8]) -> Result<u64, CompileError> {
    if machine_code.len() % RV64_INSTRUCTION_SIZE as usize != 0 {
        return Err(CompileError::Link {
            description: "linked code is not a sequence of 32-bit instructions".to_owned(),
        });
    }
    Ok((machine_code.len() / RV64_INSTRUCTION_SIZE as usize) as u64)
}

fn instruction_at(machine_code: &[u8], offset: usize) -> Result<u32, CompileError> {
    machine_code
        .get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| CompileError::Link {
            description: format!("arithmetic link offset {offset} is outside generated code"),
        })
}

fn relative_jal_offset(from_pc: u64, target_pc: u64) -> Result<i64, CompileError> {
    let offset = i64::try_from(i128::from(target_pc) - i128::from(from_pc)).map_err(|_| {
        CompileError::Link {
            description: "JAL offset does not fit an i64".to_owned(),
        }
    })?;
    encode_jal_zero(offset)?;
    Ok(offset)
}

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

//! RV64 arithmetic execution helpers shared by compiler and conformance tests.
//!
//! This module executes generated machine code through the RV64 executor
//! boundary; it does not execute EVM bytecode in a reference engine.

use alloy_primitives::U256;
use asm_rs::{assemble, Arch};

use crate::{
    rv64::executor::{IntegerRegister, Rv64Executor, RvemuExecutor},
    Instruction, Opcode,
};

const HALT_INSTRUCTION: u32 = 0x0010_0073;
const MAX_INSTRUCTIONS: usize = 256;
const OPERAND_A: u64 = RvemuExecutor::DRAM_BASE + 0x1000;
const OPERAND_B: u64 = RvemuExecutor::DRAM_BASE + 0x1020;
const RESULT: u64 = RvemuExecutor::DRAM_BASE + 0x1040;

pub const DEFAULT_RANDOM_SEED: u64 = 0x1234_5678_9abc_def0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstructionMetrics {
    pub total: u64,
    pub loads: u64,
    pub stores: u64,
    pub arithmetic: u64,
    pub comparisons: u64,
    pub logical: u64,
    pub control_flow: u64,
    pub other: u64,
}

impl InstructionMetrics {
    pub fn record(&mut self, instruction: u32) {
        self.total += 1;
        match classify_instruction(instruction) {
            InstructionCategory::Loads => self.loads += 1,
            InstructionCategory::Stores => self.stores += 1,
            InstructionCategory::Arithmetic => self.arithmetic += 1,
            InstructionCategory::Comparisons => self.comparisons += 1,
            InstructionCategory::Logical => self.logical += 1,
            InstructionCategory::ControlFlow => self.control_flow += 1,
            InstructionCategory::Other => self.other += 1,
        }
    }

    pub fn category_sum(self) -> u64 {
        self.loads
            + self.stores
            + self.arithmetic
            + self.comparisons
            + self.logical
            + self.control_flow
            + self.other
    }

    pub fn add_assign(&mut self, other: Self) {
        self.total += other.total;
        self.loads += other.loads;
        self.stores += other.stores;
        self.arithmetic += other.arithmetic;
        self.comparisons += other.comparisons;
        self.logical += other.logical;
        self.control_flow += other.control_flow;
        self.other += other.other;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstructionCategory {
    Loads,
    Stores,
    Arithmetic,
    Comparisons,
    Logical,
    ControlFlow,
    Other,
}

/// Categorizes a decoded base 32-bit RISC-V instruction.
///
/// The categories are mutually exclusive.  Loads/stores and control flow are
/// identified by their opcode.  Integer ADD/SUB and multiply/divide families
/// are arithmetic; SLT/SLTU are comparisons; bitwise operations and shifts are
/// logical.  Any unrecognized encoding is intentionally retained as `other`.
fn classify_instruction(instruction: u32) -> InstructionCategory {
    let opcode = instruction & 0x7f;
    let funct3 = (instruction >> 12) & 0x7;
    let funct7 = (instruction >> 25) & 0x7f;
    match opcode {
        0x03 => InstructionCategory::Loads,
        0x23 => InstructionCategory::Stores,
        0x63 | 0x67 | 0x6f => InstructionCategory::ControlFlow,
        // OP-IMM: ADDI is arithmetic; SLTI/SLTIU are comparisons; the
        // remaining immediate operations are bitwise/shift logical operations.
        0x13 | 0x1b => match funct3 {
            0 => InstructionCategory::Arithmetic,
            2 | 3 => InstructionCategory::Comparisons,
            1 | 4 | 5 | 6 | 7 => InstructionCategory::Logical,
            _ => InstructionCategory::Other,
        },
        // OP and OP-32 contain the regular integer and M-extension forms.
        // Their funct3 layouts make SLT/SLTU the only comparison operations.
        0x33 | 0x3b if funct7 == 0x01 => InstructionCategory::Arithmetic,
        0x33 | 0x3b => match funct3 {
            0 => InstructionCategory::Arithmetic,
            2 | 3 => InstructionCategory::Comparisons,
            1 | 4 | 5 | 6 | 7 => InstructionCategory::Logical,
            _ => InstructionCategory::Other,
        },
        _ => InstructionCategory::Other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rv64Execution {
    pub result: U256,
    pub metrics: InstructionMetrics,
}

pub struct Rv64Runner {
    executor: RvemuExecutor,
    halt_pc: u64,
    machine_code_hash: u64,
    routine_instruction_words: u64,
    routine_metrics: InstructionMetrics,
}

impl Rv64Runner {
    pub fn new(opcode: Opcode) -> Result<Self, String> {
        let program = assemble_generated(opcode)?;
        let halt_pc = RvemuExecutor::DRAM_BASE + program.len() as u64 - 4;
        let routine_len = program.len() - 4;
        let machine_code_hash = fnv1a64(&program[..routine_len]);
        let routine_metrics = metrics_for_instruction_words(&program[..routine_len]);
        let mut executor = RvemuExecutor::new()
            .map_err(|error| format!("could not initialize RV64 executor: {error}"))?;
        executor
            .load_code(&program)
            .map_err(|error| format!("could not load RV64 machine code: {error}"))?;
        Ok(Self {
            executor,
            halt_pc,
            machine_code_hash,
            routine_instruction_words: (routine_len / 4) as u64,
            routine_metrics,
        })
    }

    /// FNV-1a 64-bit hash of the generated routine's encoded instructions.
    /// The harness EBREAK sentinel is excluded.
    pub fn machine_code_hash(&self) -> u64 {
        self.machine_code_hash
    }

    pub fn routine_instruction_words(&self) -> u64 {
        self.routine_instruction_words
    }

    /// Executes decoded machine instructions until the generated function
    /// returns to the harness sentinel.  The sentinel itself is not executed
    /// or counted.
    pub fn run(&mut self, a: U256, b: U256) -> Result<Rv64Execution, String> {
        for (address, value) in [(OPERAND_A, a), (OPERAND_B, b)] {
            self.executor
                .write_memory(address, &u256_to_little_endian_limbs(value))
                .map_err(|error| format!("could not initialize RV64 operand memory: {error}"))?;
        }

        self.executor
            .write_register(IntegerRegister::RETURN_ADDRESS, self.halt_pc);
        self.executor.write_register(
            IntegerRegister::new(10).expect("a0 is a valid register"),
            OPERAND_A,
        );
        self.executor.write_register(
            IntegerRegister::new(11).expect("a1 is a valid register"),
            OPERAND_B,
        );
        self.executor.write_register(
            IntegerRegister::new(12).expect("a2 is a valid register"),
            RESULT,
        );

        let report = self
            .executor
            .execute(
                RvemuExecutor::DRAM_BASE,
                self.halt_pc,
                MAX_INSTRUCTIONS as u64,
            )
            .map_err(|error| format!("RV64 execution stopped before EBREAK: {error}"))?;
        if report.executed_instructions != self.routine_metrics.total {
            return Err(format!(
                "RV64 execution reached EBREAK after {} instructions; expected {} from the generated routine",
                report.executed_instructions, self.routine_metrics.total
            ));
        }

        let mut result_bytes = [0; 32];
        let mut result_memory = [0; 32];
        self.executor
            .read_memory(RESULT, &mut result_memory)
            .map_err(|error| format!("could not read RV64 result memory: {error}"))?;
        for index in 0..4 {
            let start = index * 8;
            let limb = u64::from_le_bytes(
                result_memory[start..start + 8]
                    .try_into()
                    .expect("fixed-size result limb"),
            );
            let start = 24 - (index * 8);
            result_bytes[start..start + 8].copy_from_slice(&limb.to_be_bytes());
        }
        Ok(Rv64Execution {
            result: U256::from_be_bytes(result_bytes),
            metrics: self.routine_metrics,
        })
    }
}

/// Executes decoded arithmetic instructions with a host-managed stack and RV64 arithmetic.
///
/// This legacy helper deliberately rejects POP and DUP1: stack manipulation
/// is implemented only by the stack-aware guest RV64 path, never by host-side
/// stack mutation.
pub struct ProgramRunner {
    add: Rv64Runner,
    sub: Rv64Runner,
}

impl ProgramRunner {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            add: Rv64Runner::new(Opcode::Add)?,
            sub: Rv64Runner::new(Opcode::Sub)?,
        })
    }

    /// Returns the top stack value, or an error for underflow or an empty stack.
    pub fn run(&mut self, program: &[Instruction]) -> Result<U256, String> {
        let mut stack = Vec::new();
        for (index, instruction) in program.iter().enumerate() {
            match instruction {
                Instruction::Stop => break,
                Instruction::Push(value) => stack.push(*value),
                Instruction::Pop => {
                    return Err("POP requires the stack-aware RV64 guest execution path".to_owned())
                }
                Instruction::Dup1 => {
                    return Err("DUP1 requires the stack-aware RV64 guest execution path".to_owned())
                }
                Instruction::Add | Instruction::Sub => {
                    if stack.len() < 2 {
                        return Err(format!("stack underflow at instruction {index}"));
                    }
                    let a = stack.pop().expect("checked stack depth");
                    let b = stack.pop().expect("checked stack depth");
                    let runner = match instruction {
                        Instruction::Add => &mut self.add,
                        Instruction::Sub => &mut self.sub,
                        Instruction::Stop
                        | Instruction::Push(_)
                        | Instruction::Pop
                        | Instruction::Dup1 => unreachable!(),
                    };
                    stack.push(runner.run(a, b)?.result);
                }
            }
        }
        stack.pop().ok_or_else(|| "empty result stack".to_owned())
    }
}
#[derive(Debug, Clone, Copy)]
pub struct OperandCase {
    pub label: &'static str,
    pub a: U256,
    pub b: U256,
}

/// Shared directed input pairs. They are intentionally used for both ADD and
/// SUB so an ALL run exercises the same vectors in both operations.
pub fn directed_cases() -> Vec<OperandCase> {
    vec![
        OperandCase {
            label: "zero",
            a: U256::ZERO,
            b: U256::ZERO,
        },
        OperandCase {
            label: "simple arithmetic",
            a: U256::from(5),
            b: U256::from(7),
        },
        OperandCase {
            label: "maximum and zero",
            a: U256::MAX,
            b: U256::ZERO,
        },
        OperandCase {
            label: "256-bit wraparound",
            a: U256::MAX,
            b: U256::from(1),
        },
        OperandCase {
            label: "zero minus one wraparound",
            a: U256::ZERO,
            b: U256::from(1),
        },
        OperandCase {
            label: "carry across limb boundary",
            a: from_limbs([u64::MAX, 0, 0, 0]),
            b: U256::from(1),
        },
        OperandCase {
            label: "borrow across limb boundary",
            a: from_limbs([0, 1, 0, 0]),
            b: U256::from(1),
        },
        OperandCase {
            label: "carry across multiple limbs",
            a: from_limbs([u64::MAX, u64::MAX, 0, 0]),
            b: U256::from(1),
        },
        OperandCase {
            label: "borrow across multiple limbs",
            a: from_limbs([0, 0, 1, 0]),
            b: U256::from(1),
        },
        OperandCase {
            label: "carry propagation across multiple limbs",
            a: from_limbs([u64::MAX, u64::MAX, u64::MAX, 0]),
            b: U256::from(1),
        },
        OperandCase {
            label: "borrow propagation across multiple limbs",
            a: from_limbs([0, 0, 0, 1]),
            b: U256::from(1),
        },
        OperandCase {
            label: "carry propagation across all limbs",
            a: U256::MAX,
            b: from_limbs([1, 1, 1, 1]),
        },
    ]
}

pub fn random_u256(state: &mut u64) -> U256 {
    let mut bytes = [0; 32];
    for chunk in bytes.as_chunks_mut::<8>().0 {
        chunk.copy_from_slice(&next_random(state).to_be_bytes());
    }
    U256::from_be_bytes(bytes)
}

pub fn hex(value: U256) -> String {
    format!("0x{value:064x}")
}

fn assemble_generated(opcode: Opcode) -> Result<Vec<u8>, String> {
    let generated = format!(
        "{}\n.globl __rv64_halt\n__rv64_halt:\n    ebreak\n",
        opcode.assembly()
    );
    let assembly = generated
        .lines()
        .filter(|line| !line.trim_start().starts_with('.'))
        .collect::<Vec<_>>()
        .join("\n");
    let program = assemble(&assembly, Arch::Rv64)
        .map_err(|error| format!("could not assemble generated code for RV64IM: {error}"))?;
    if program.len() < 4 || program.len() % 4 != 0 {
        return Err(format!(
            "assembled program has invalid RV64IM text length: {} bytes",
            program.len()
        ));
    }
    let halt = u32::from_le_bytes(
        program[program.len() - 4..]
            .try_into()
            .expect("checked four-byte halt instruction"),
    );
    if halt != HALT_INSTRUCTION {
        return Err(format!(
            "assembled program does not end with EBREAK (found 0x{halt:08x})"
        ));
    }
    if program.len() as u64 >= OPERAND_A - RvemuExecutor::DRAM_BASE {
        return Err("assembled program overlaps the operand memory region".to_owned());
    }
    Ok(program)
}

fn to_limbs(value: U256) -> [u64; 4] {
    let bytes = value.to_be_bytes::<32>();
    std::array::from_fn(|index| {
        let start = 24 - index * 8;
        u64::from_be_bytes(bytes[start..start + 8].try_into().expect("8-byte limb"))
    })
}

fn u256_to_little_endian_limbs(value: U256) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, limb) in to_limbs(value).into_iter().enumerate() {
        let start = index * 8;
        bytes[start..start + 8].copy_from_slice(&limb.to_le_bytes());
    }
    bytes
}

fn metrics_for_instruction_words(machine_code: &[u8]) -> InstructionMetrics {
    let mut metrics = InstructionMetrics::default();
    for word in machine_code.as_chunks::<4>().0 {
        metrics.record(u32::from_le_bytes(*word));
    }
    metrics
}

fn from_limbs(limbs: [u64; 4]) -> U256 {
    let mut bytes = [0; 32];
    for (index, limb) in limbs.into_iter().enumerate() {
        let start = 24 - index * 8;
        bytes[start..start + 8].copy_from_slice(&limb.to_be_bytes());
    }
    U256::from_be_bytes(bytes)
}

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

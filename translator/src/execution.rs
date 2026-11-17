//! Shared differential execution support for tests and the benchmark command.
//!
//! This module deliberately executes the generated RV64 machine code in rvemu
//! and the EVM bytecode in revm.  It does not use the Rust limb helpers as a
//! reference for either path.

use asm_rs::{assemble, Arch};
use revm::{
    db::InMemoryDB,
    primitives::{
        AccountInfo, Address, Bytecode, ExecutionResult, Output, SpecId, TransactTo, U256,
    },
    Evm,
};
use rvemu::{bus::DRAM_BASE, emulator::Emulator};
use std::{env, fs, path::PathBuf, sync::Mutex};

use crate::Opcode;

const HALT_INSTRUCTION: u32 = 0x0010_0073;
const MAX_INSTRUCTIONS: usize = 256;
const OPERAND_A: u64 = DRAM_BASE + 0x1000;
const OPERAND_B: u64 = DRAM_BASE + 0x1020;
const RESULT: u64 = DRAM_BASE + 0x1040;

// rvemu writes its generated device-tree files relative to the process working
// directory during `Emulator::new`. Construction is serialized while the
// current directory is temporarily changed so those side effects stay in the
// ignored translator/runs directory, including when integration tests run in
// parallel.
static RVEMU_INITIALIZATION_LOCK: Mutex<()> = Mutex::new(());

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
    emulator: Emulator,
    halt_pc: u64,
    machine_code_hash: u64,
    routine_instruction_words: u64,
}

impl Rv64Runner {
    pub fn new(opcode: Opcode) -> Result<Self, String> {
        let program = assemble_generated(opcode)?;
        let halt_pc = DRAM_BASE + program.len() as u64 - 4;
        let routine_len = program.len() - 4;
        let machine_code_hash = fnv1a64(&program[..routine_len]);
        let mut emulator = new_emulator_in_artifact_directory()?;
        emulator.initialize_dram(program);
        Ok(Self {
            emulator,
            halt_pc,
            machine_code_hash,
            routine_instruction_words: (routine_len / 4) as u64,
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
            for (index, limb) in to_limbs(value).into_iter().enumerate() {
                self.emulator
                    .cpu
                    .bus
                    .write(address + (index as u64 * 8), limb, 64)
                    .map_err(|error| {
                        format!("could not initialize RV64 operand memory: {error:?}")
                    })?;
            }
        }

        self.emulator.cpu.xregs.write(1, self.halt_pc);
        self.emulator.cpu.xregs.write(10, OPERAND_A);
        self.emulator.cpu.xregs.write(11, OPERAND_B);
        self.emulator.cpu.xregs.write(12, RESULT);
        self.emulator.initialize_pc(DRAM_BASE);

        let mut metrics = InstructionMetrics::default();
        let mut halted = false;
        for _ in 0..MAX_INSTRUCTIONS {
            if self.emulator.cpu.pc == self.halt_pc {
                halted = true;
                break;
            }
            // Read and decode the exact 32-bit word at the PC immediately
            // before rvemu executes it. This accounts for assembler-expanded
            // pseudoinstructions and follows actual control flow.
            let instruction = self
                .emulator
                .cpu
                .bus
                .read(self.emulator.cpu.pc, 32)
                .map_err(|error| format!("could not fetch RV64 instruction: {error:?}"))?
                as u32;
            self.emulator
                .cpu
                .execute()
                .map_err(|error| format!("RV64 execution stopped before EBREAK: {error:?}"))?;
            metrics.record(instruction);
        }
        if !halted {
            return Err(format!(
                "RV64 execution did not return to EBREAK within {MAX_INSTRUCTIONS} instructions"
            ));
        }

        let mut result_bytes = [0; 32];
        for index in 0..4 {
            let limb = self
                .emulator
                .cpu
                .bus
                .read(RESULT + (index as u64 * 8), 64)
                .map_err(|error| format!("could not read RV64 result memory: {error:?}"))?;
            let start = 24 - (index * 8);
            result_bytes[start..start + 8].copy_from_slice(&limb.to_be_bytes());
        }
        Ok(Rv64Execution {
            result: U256::from_be_bytes(result_bytes),
            metrics,
        })
    }
}

fn new_emulator_in_artifact_directory() -> Result<Emulator, String> {
    let _lock = RVEMU_INITIALIZATION_LOCK
        .lock()
        .map_err(|_| "RV64 emulator initialization lock was poisoned".to_owned())?;
    let artifact_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runs");
    fs::create_dir_all(&artifact_directory).map_err(|error| {
        format!(
            "could not create rvemu artifact directory {}: {error}",
            artifact_directory.display()
        )
    })?;
    let original_directory = env::current_dir()
        .map_err(|error| format!("could not read process working directory: {error}"))?;
    env::set_current_dir(&artifact_directory).map_err(|error| {
        format!(
            "could not enter rvemu artifact directory {}: {error}",
            artifact_directory.display()
        )
    })?;
    let restore = WorkingDirectoryRestore { original_directory };
    let emulator = Emulator::new();
    drop(restore);
    Ok(emulator)
}

struct WorkingDirectoryRestore {
    original_directory: PathBuf,
}

impl Drop for WorkingDirectoryRestore {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original_directory);
    }
}

pub fn run_evm(opcode: Opcode, a: U256, b: U256) -> Result<U256, String> {
    let mut bytecode = Vec::with_capacity(2 * 33 + 9);
    // EVM binary operations pop the top item first. SUB therefore needs b
    // below a on the stack to evaluate a - b; ADD's order is immaterial.
    let operands = match opcode {
        Opcode::Add => [a, b],
        Opcode::Sub => [b, a],
    };
    for operand in operands {
        bytecode.push(0x7f);
        bytecode.extend_from_slice(&operand.to_be_bytes::<32>());
    }
    bytecode.extend_from_slice(&[
        opcode.evm_byte(),
        0x60,
        0x00,
        0x52, // PUSH1 0; MSTORE
        0x60,
        0x20,
        0x60,
        0x00,
        0xf3, // PUSH1 32; PUSH1 0; RETURN
    ]);

    run_evm_bytecode(bytecode)
}

/// Executes bytecode in the same fixed environment used by the arithmetic
/// differential tests and returns its 32-byte call output.
pub fn run_evm_bytecode(bytecode: Vec<u8>) -> Result<U256, String> {
    let contract = Address::from([0x22; 20]);
    let caller = Address::from([0x11; 20]);
    let mut database = InMemoryDB::default();
    database.insert_account_info(
        contract,
        AccountInfo::from_bytecode(Bytecode::new_raw(bytecode.into())),
    );
    let mut evm = Evm::builder()
        .with_db(database)
        .with_spec_id(SpecId::CANCUN)
        .modify_tx_env(|tx| {
            tx.caller = caller;
            tx.transact_to = TransactTo::Call(contract);
            tx.gas_limit = 1_000_000;
        })
        .build();
    let result = evm
        .transact_preverified()
        .map_err(|error| format!("revm transaction failed: {error:?}"))?
        .result;
    match result {
        ExecutionResult::Success {
            output: Output::Call(output),
            ..
        } if output.len() == 32 => {
            let bytes: [u8; 32] = output
                .as_ref()
                .try_into()
                .map_err(|_| "revm returned an output other than 32 bytes".to_owned())?;
            Ok(U256::from_be_bytes(bytes))
        }
        other => Err(format!("revm did not return a 32-byte result: {other:?}")),
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
    for chunk in bytes.chunks_exact_mut(8) {
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
    if program.len() as u64 >= OPERAND_A - DRAM_BASE {
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

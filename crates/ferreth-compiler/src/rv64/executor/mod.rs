//! Backend-neutral boundary for executing RV64 guest code.

mod rvemu;

use std::fmt;

pub use rvemu::RvemuExecutor;

/// An RV64 integer-register index.
///
/// This type deliberately represents only the architectural register number,
/// rather than exposing a backend register representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntegerRegister(u8);

impl IntegerRegister {
    /// Creates an integer-register index in the architectural range `x0..x31`.
    pub fn new(index: u8) -> Result<Self, ExecutorError> {
        if index < 32 {
            Ok(Self(index))
        } else {
            Err(ExecutorError::InvalidRegister { index })
        }
    }

    pub const ZERO: Self = Self(0);
    pub const RETURN_ADDRESS: Self = Self(1);
    /// Native RISC-V stack pointer (`sp`, x2).
    pub const STACK_POINTER: Self = Self(2);
    /// EVM guest operand-stack base (`s0`, x8).
    pub const EVM_STACK_BASE: Self = Self(8);
    /// EVM guest next-free operand-stack pointer (`s1`, x9).
    pub const EVM_STACK_NEXT_FREE: Self = Self(9);

    pub fn index(self) -> u8 {
        self.0
    }
}

/// Observation that an RV64 run reached a caller-selected stop PC.
///
/// Callers can construct reports directly. The fields carry no proof of
/// provenance or EVM semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionReport {
    /// Instructions for which the backend completed `execute()` successfully.
    /// The configured exit-PC instruction is never included.
    pub executed_instructions: u64,
    /// Configured exit PC observed before its next instruction fetch.
    pub exit_pc: u64,
}

/// Errors raised by the executor boundary.
///
/// Normal completion is represented exclusively by `Ok(ExecutionReport)`.
/// In particular, an instruction limit and a backend execution fault remain
/// distinguishable from successful arrival at the configured exit PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorError {
    InvalidRegister {
        index: u8,
    },
    MemoryOutOfRange {
        address: u64,
        width: usize,
    },
    InvalidInstructionAddress {
        address: u64,
    },
    InstructionLimitExceeded {
        limit: u64,
        executed_instructions: u64,
    },
    Initialization {
        description: String,
    },
    MemoryAccess {
        address: u64,
        width: usize,
        description: String,
    },
    ExecutionFault {
        executed_instructions: u64,
        description: String,
    },
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegister { index } => write!(f, "invalid RV64 integer register x{index}"),
            Self::MemoryOutOfRange { address, width } => write!(
                f,
                "guest memory access [0x{address:016x}, 0x{:016x}) with width {width} is outside executor DRAM",
                address.saturating_add(*width as u64)
            ),
            Self::InvalidInstructionAddress { address } => {
                write!(f, "RV64 instruction address 0x{address:016x} is not four-byte aligned")
            }
            Self::InstructionLimitExceeded {
                limit,
                executed_instructions,
            } => write!(
                f,
                "RV64 instruction limit of {limit} reached after {executed_instructions} successful instructions"
            ),
            Self::Initialization { description } => {
                write!(f, "could not initialize RV64 executor: {description}")
            }
            Self::MemoryAccess {
                address,
                width,
                description,
            } => write!(
                f,
                "RV64 guest-memory access at 0x{address:016x} with width {width} failed: {description}"
            ),
            Self::ExecutionFault {
                executed_instructions,
                description,
            } => {
                write!(
                    f,
                    "RV64 execution fault after {executed_instructions} successful instructions: {description}"
                )
            }
        }
    }
}

impl std::error::Error for ExecutorError {}

/// Low-level contract for loading, running, and observing RV64 code.
///
/// The caller supplies the machine image, initial state, stop PCs, and RV64
/// instruction bound. The executor performs no EVM image verification or
/// frame gas accounting. Use [`crate::orchestrator::execute_guest_evm_bytecode`]
/// to run a verified metered guest through the canonical path.
pub trait Rv64Executor {
    /// Loads assembled machine code at the backend's RV64 code base.
    fn load_code(&mut self, machine_code: &[u8]) -> Result<(), ExecutorError>;

    /// Writes bytes to guest memory after validating the complete byte range.
    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), ExecutorError>;

    /// Reads bytes from guest memory after validating the complete byte range.
    fn read_memory(&mut self, address: u64, bytes: &mut [u8]) -> Result<(), ExecutorError>;

    /// Writes an architectural RV64 integer register.
    fn write_register(&mut self, register: IntegerRegister, value: u64);

    /// Reads an architectural RV64 integer register.
    fn read_register(&mut self, register: IntegerRegister) -> u64;

    /// Executes from `entry_pc` until `exit_pc` is observed before a fetch.
    fn execute(
        &mut self,
        entry_pc: u64,
        exit_pc: u64,
        instruction_limit: u64,
    ) -> Result<ExecutionReport, ExecutorError>;

    /// Executes until one of `exit_pcs` is observed before an instruction
    /// fetch. This permits callers to measure a dynamically reached boundary
    /// without treating the boundary instruction itself as executed.
    fn execute_until(
        &mut self,
        entry_pc: u64,
        exit_pcs: &[u64],
        instruction_limit: u64,
    ) -> Result<ExecutionReport, ExecutorError>;
}

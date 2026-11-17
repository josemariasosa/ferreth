//! Minimal Rust orchestration for isolated RV64 guest opcode experiments.
//!
//! This module deliberately links only the code needed to run one `PUSH1`.
//! The opcode body remains owned by [`crate::rv64::push`]; publication and
//! exit code are runtime infrastructure that future isolated opcodes can
//! share without putting termination policy in their generators.

use std::fmt;

use revm::primitives::U256;

use crate::{
    compiler::{compile_evm_bytecode, compile_push1_program, CompileError, CompiledRv64Program},
    rv64::{
        executor::{ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{
            initialize_guest_state, read_guest_result, write_evm_word, GuestExecutionStatus,
            GuestInitializationError, GuestMemoryError, GuestResult, GuestResultError,
        },
        memory::{LayoutError, ENTRY_PC, EVM_STACK_CAPACITY, EVM_STACK_REGION, EXIT_PC},
    },
};

/// rvemu version used by this initial isolated-opcode orchestrator.
pub const EXECUTOR_IDENTITY: &str = "rvemu 0.0.11";

/// Explicitly provisional guest exception code for a rejected EVM stack push.
///
/// The accepted ABI reserves the exception-code field but not its production
/// taxonomy.  Keep this small isolated constant until that taxonomy is
/// decided, rather than treating it as a stable EVM error code.
const EBREAK: u32 = 0x0010_0073;

/// Dynamic RV64 instruction counts for one isolated guest execution.
///
/// These are successfully executed RV64 instructions, not EVM gas, elapsed
/// time, or hardware cycles. `opcode_body` includes the linked overflow
/// trampoline when that path is taken, because it occupies the generated
/// opcode-body range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestInstructionMetrics {
    opcode_body: u64,
    guest_termination: u64,
    total: u64,
}

impl GuestInstructionMetrics {
    pub const fn opcode_body(self) -> u64 {
        self.opcode_body
    }

    pub const fn guest_termination(self) -> u64 {
        self.guest_termination
    }

    pub const fn total(self) -> u64 {
        self.total
    }
}

/// Host-visible result of one linked, isolated guest `PUSH1` execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestratorResult {
    assembly: String,
    machine_code: Vec<u8>,
    guest_result: GuestResult,
    instruction_metrics: GuestInstructionMetrics,
    executor_identity: &'static str,
}

impl OrchestratorResult {
    /// Complete linked RV64 assembly, including the runtime exit handlers.
    pub fn assembly(&self) -> &str {
        &self.assembly
    }

    /// Complete linked RV64 instruction bytes in increasing address order.
    pub fn machine_code(&self) -> &[u8] {
        &self.machine_code
    }

    /// Guest-published terminal result retrieved after executor-observed exit.
    pub fn guest_result(&self) -> &GuestResult {
        &self.guest_result
    }

    pub const fn instruction_metrics(&self) -> GuestInstructionMetrics {
        self.instruction_metrics
    }

    pub const fn executor_identity(&self) -> &'static str {
        self.executor_identity
    }
}

/// Failures in the isolated-opcode orchestration boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorError {
    Layout(LayoutError),
    Executor(ExecutorError),
    GuestInitialization(GuestInitializationError),
    GuestMemory(GuestMemoryError),
    GuestResult(GuestResultError),
    Compile(CompileError),
    InitialStackTooDeep { depth: usize },
    Link { description: String },
    ExecutionAccounting { total: u64, termination: u64 },
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "invalid linked guest layout: {error}"),
            Self::Executor(error) => write!(f, "RV64 executor failure: {error}"),
            Self::GuestInitialization(error) => write!(f, "could not initialize guest: {error}"),
            Self::GuestMemory(error) => write!(f, "could not prepare guest stack: {error}"),
            Self::GuestResult(error) => write!(f, "could not retrieve guest result: {error}"),
            Self::Compile(error) => write!(f, "could not compile PUSH1 program: {error}"),
            Self::InitialStackTooDeep { depth } => write!(
                f,
                "initial EVM stack depth {depth} exceeds capacity {EVM_STACK_CAPACITY}"
            ),
            Self::Link { description } => {
                write!(f, "could not link isolated PUSH1 program: {description}")
            }
            Self::ExecutionAccounting { total, termination } => write!(
                f,
                "executed instruction total {total} is below termination count {termination}"
            ),
        }
    }
}

impl std::error::Error for OrchestratorError {}

impl From<LayoutError> for OrchestratorError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<ExecutorError> for OrchestratorError {
    fn from(error: ExecutorError) -> Self {
        Self::Executor(error)
    }
}

impl From<GuestInitializationError> for OrchestratorError {
    fn from(error: GuestInitializationError) -> Self {
        Self::GuestInitialization(error)
    }
}

impl From<GuestMemoryError> for OrchestratorError {
    fn from(error: GuestMemoryError) -> Self {
        Self::GuestMemory(error)
    }
}

impl From<GuestResultError> for OrchestratorError {
    fn from(error: GuestResultError) -> Self {
        Self::GuestResult(error)
    }
}

impl From<CompileError> for OrchestratorError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

/// Executes one isolated EVM `PUSH1` through the RV64 guest ABI.
///
/// The host starts with an empty EVM stack. The returned program presentation
/// is the linked program actually loaded into rvemu, not a separate display
/// encoding of the opcode generator.
pub fn execute_guest_push1(immediate: u8) -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_push1_sequence(&[immediate])
}

/// Compiles and executes a complete sequence of `PUSH1` operations once.
pub fn execute_guest_push1_sequence(
    immediates: &[u8],
) -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_push1_sequence_with_initial_stack(immediates, &[])
}

/// Shared execution path with host-provided valid initial stack words.
///
/// This is intentionally limited to PUSH1 sequences so tests can construct a
/// full-stack ABI entry state without emulating any EVM opcode on the host.
pub fn execute_guest_push1_sequence_with_initial_stack(
    immediates: &[u8],
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    let linked = compile_push1_program(immediates)?;
    execute_linked_program(linked, initial_stack)
}

/// Decodes, compiles, and executes a PUSH1-only EVM bytecode program once.
///
/// The executor runs the resulting RV64 executable as a whole; Rust does not
/// execute individual EVM operations.
pub fn execute_guest_evm_bytecode(
    bytecode: &[u8],
) -> Result<OrchestratorResult, OrchestratorError> {
    execute_linked_program(compile_evm_bytecode(bytecode)?, &[])
}

fn execute_linked_program(
    linked: CompiledRv64Program,
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    if initial_stack.len() > EVM_STACK_CAPACITY as usize {
        return Err(OrchestratorError::InitialStackTooDeep {
            depth: initial_stack.len(),
        });
    }

    let mut executor = RvemuExecutor::new()?;
    executor.load_code(linked.machine_code())?;
    // This makes an accidental fetch of the reserved exit slot observable as
    // an executor fault. A valid run is instead stopped by `execute` before
    // this instruction is fetched.
    executor.write_memory(EXIT_PC, &EBREAK.to_le_bytes())?;
    initialize_guest_state(&mut executor)?;

    for (slot, word) in initial_stack.iter().copied().enumerate() {
        write_evm_word(&mut executor, slot, word)?;
    }
    let initial_s1 = EVM_STACK_REGION.start()
        + u64::try_from(initial_stack.len()).expect("stack capacity fits u64") * 32;
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, initial_s1);

    let report = executor.execute(ENTRY_PC, EXIT_PC, linked.instruction_limit())?;
    let guest_result = read_guest_result(&mut executor, &report)?;
    let guest_termination = match guest_result.status() {
        GuestExecutionStatus::Normal => linked.normal_termination_instructions(),
        GuestExecutionStatus::GuestException => linked.exceptional_termination_instructions(),
        GuestExecutionStatus::Running => unreachable!("guest result validation rejects RUNNING"),
    };
    let opcode_body = report
        .executed_instructions
        .checked_sub(guest_termination)
        .ok_or(OrchestratorError::ExecutionAccounting {
            total: report.executed_instructions,
            termination: guest_termination,
        })?;

    Ok(OrchestratorResult {
        assembly: linked.assembly().to_owned(),
        machine_code: linked.machine_code().to_vec(),
        guest_result,
        instruction_metrics: GuestInstructionMetrics {
            opcode_body,
            guest_termination,
            total: report.executed_instructions,
        },
        executor_identity: EXECUTOR_IDENTITY,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_stack_overflow_publishes_an_exception_without_mutating_the_stack() {
        let initial_stack = (0..EVM_STACK_CAPACITY)
            .map(|slot| U256::from(slot + 1))
            .collect::<Vec<_>>();
        let result = execute_guest_push1_sequence_with_initial_stack(&[5], &initial_stack)
            .expect("full-stack PUSH1 reaches the guest exception exit");

        assert_eq!(
            result.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(
            result.guest_result().exception_code(),
            crate::compiler::PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE
        );
        assert_eq!(result.guest_result().final_s1(), EVM_STACK_REGION.end());
        assert_eq!(result.guest_result().stack(), initial_stack);
        assert_eq!(result.instruction_metrics().opcode_body(), 4);
        assert_eq!(result.instruction_metrics().guest_termination(), 8);
        assert_eq!(result.instruction_metrics().total(), 12);
    }

    #[test]
    fn push1_normal_execution_publishes_rv64_stack_results() {
        for immediate in [0, 5, u8::MAX] {
            let result = execute_guest_push1(immediate).expect("PUSH1 completes through RV64");

            assert_eq!(result.guest_result().status(), GuestExecutionStatus::Normal);
            assert_eq!(result.guest_result().exception_code(), 0);
            assert_eq!(result.guest_result().stack(), &[U256::from(immediate)]);
            assert_eq!(
                result.guest_result().final_s1(),
                EVM_STACK_REGION.start() + 32
            );
            assert_eq!(result.instruction_metrics().opcode_body(), 10);
            assert_eq!(result.instruction_metrics().guest_termination(), 7);
            assert_eq!(result.instruction_metrics().total(), 17);
        }
    }

    #[test]
    fn linked_assembly_reassembles_to_the_loaded_machine_code() {
        let linked = compile_push1_program(&[5]).expect("link PUSH1");
        assert_eq!(
            asm_rs::assemble(linked.assembly(), asm_rs::Arch::Rv64)
                .expect("reassemble linked source"),
            linked.machine_code()
        );
        assert!(!linked.assembly().contains("jal zero, 0"));
    }
}

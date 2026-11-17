//! Minimal Rust orchestration for isolated RV64 guest opcode experiments.
//!
//! This module deliberately links only the code needed to run one `PUSH1`.
//! The opcode body remains owned by [`crate::rv64::push`]; publication and
//! exit code are runtime infrastructure that future isolated opcodes can
//! share without putting termination policy in their generators.

use std::fmt;

use asm_rs::{assemble, Arch};
use revm::primitives::U256;

use crate::rv64::{
    executor::{ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor},
    guest::{
        initialize_guest_state, read_guest_result, write_evm_word, GuestExecutionStatus,
        GuestInitializationError, GuestMemoryError, GuestResult, GuestResultError,
    },
    memory::{
        validate_generated_code, LayoutError, ENTRY_PC, EVM_STACK_CAPACITY, EVM_STACK_REGION,
        EXIT_PC, RV64_INSTRUCTION_SIZE,
    },
    push::generate_guest_push1,
};

/// rvemu version used by this initial isolated-opcode orchestrator.
pub const EXECUTOR_IDENTITY: &str = "rvemu 0.0.11";

/// Explicitly provisional guest exception code for a rejected EVM stack push.
///
/// The accepted ABI reserves the exception-code field but not its production
/// taxonomy.  Keep this small isolated constant until that taxonomy is
/// decided, rather than treating it as a stable EVM error code.
pub const PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE: u64 = 1;

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

/// Executes one isolated EVM `PUSH1` through the RV64 guest ABI.
///
/// The host starts with an empty EVM stack. The returned program presentation
/// is the linked program actually loaded into rvemu, not a separate display
/// encoding of the opcode generator.
pub fn execute_guest_push1(immediate: u8) -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_push1_with_initial_stack(immediate, &[])
}

/// Shared execution path with host-provided initial stack words.
///
/// This remains private because the public RO interface is intentionally just
/// the empty-stack single-opcode experiment. It exists so focused tests can
/// construct a valid full-stack ABI entry state without emulating PUSH1 on the
/// host. Future isolated opcodes can adopt a general setup API when needed.
fn execute_guest_push1_with_initial_stack(
    immediate: u8,
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    if initial_stack.len() > EVM_STACK_CAPACITY as usize {
        return Err(OrchestratorError::InitialStackTooDeep {
            depth: initial_stack.len(),
        });
    }

    let linked = link_guest_push1(immediate)?;
    let mut executor = RvemuExecutor::new()?;
    executor.load_code(&linked.machine_code)?;
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

    let report = executor.execute(ENTRY_PC, EXIT_PC, linked.instruction_limit)?;
    let guest_result = read_guest_result(&mut executor, &report)?;
    let guest_termination = linked.termination_instruction_count(guest_result.status());
    let opcode_body = report
        .executed_instructions
        .checked_sub(guest_termination)
        .ok_or(OrchestratorError::ExecutionAccounting {
            total: report.executed_instructions,
            termination: guest_termination,
        })?;

    Ok(OrchestratorResult {
        assembly: linked.assembly,
        machine_code: linked.machine_code,
        guest_result,
        instruction_metrics: GuestInstructionMetrics {
            opcode_body,
            guest_termination,
            total: report.executed_instructions,
        },
        executor_identity: EXECUTOR_IDENTITY,
    })
}

struct LinkedPush1 {
    assembly: String,
    machine_code: Vec<u8>,
    instruction_limit: u64,
    normal_termination_instructions: u64,
    exceptional_termination_instructions: u64,
}

impl LinkedPush1 {
    fn termination_instruction_count(&self, status: GuestExecutionStatus) -> u64 {
        match status {
            GuestExecutionStatus::Normal => self.normal_termination_instructions,
            GuestExecutionStatus::GuestException => self.exceptional_termination_instructions,
            GuestExecutionStatus::Running => {
                unreachable!("guest result validation rejects RUNNING")
            }
        }
    }
}

/// Links the generated opcode body to normal and PUSH1-overflow exit paths.
fn link_guest_push1(immediate: u8) -> Result<LinkedPush1, OrchestratorError> {
    let generated = generate_guest_push1(immediate);
    let opcode_len = generated.machine_code().len();
    let trampoline_offset = generated.overflow_trampoline_offset();
    let normal_template = normal_exit_assembly();
    let overflow_template = overflow_exit_assembly();
    let normal_len = assemble(&normal_template, Arch::Rv64)
        .map_err(assembly_error)?
        .len();
    let overflow_len = assemble(&overflow_template, Arch::Rv64)
        .map_err(assembly_error)?
        .len();

    let normal_start = ENTRY_PC + u64::try_from(opcode_len).expect("opcode length fits u64");
    let overflow_start = normal_start + u64::try_from(normal_len).expect("normal length fits u64");
    let trampoline_pc = ENTRY_PC + u64::try_from(trampoline_offset).expect("offset fits u64");
    let overflow_trampoline_offset = relative_jal_offset(trampoline_pc, overflow_start)?;

    let normal_assembly = normal_exit_assembly();
    let overflow_assembly = overflow_exit_assembly();
    let normal_code = assemble(&normal_assembly, Arch::Rv64).map_err(assembly_error)?;
    let overflow_code = assemble(&overflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    debug_assert_eq!(normal_code.len(), normal_len);
    debug_assert_eq!(overflow_code.len(), overflow_len);

    let trampoline_assembly = format!("    jal zero, {overflow_trampoline_offset}\n");
    let opcode_assembly = generated
        .assembly()
        .strip_suffix("    jal zero, 0\n")
        .ok_or_else(|| OrchestratorError::Link {
            description: "PUSH1 generator no longer ends in its documented trampoline".to_owned(),
        })?;
    let assembly =
        format!("{opcode_assembly}{trampoline_assembly}{normal_assembly}{overflow_assembly}");

    // Retain the existing opcode bytes and patch exactly its documented
    // trampoline; runtime handler bytes are assembled from the linked display
    // assembly. This avoids a second PUSH1 opcode implementation.
    let mut machine_code = generated.machine_code().to_vec();
    let trampoline = encode_jal_zero(overflow_trampoline_offset)?;
    machine_code[trampoline_offset..trampoline_offset + 4]
        .copy_from_slice(&trampoline.to_le_bytes());
    machine_code.extend_from_slice(&normal_code);
    machine_code.extend_from_slice(&overflow_code);

    let reassembled = assemble(&assembly, Arch::Rv64).map_err(assembly_error)?;
    if reassembled != machine_code {
        return Err(OrchestratorError::Link {
            description: "linked display assembly does not reproduce loaded machine code"
                .to_owned(),
        });
    }
    validate_generated_code(ENTRY_PC, machine_code.len())?;

    let normal_termination_instructions = instruction_count(&normal_code)?;
    let exceptional_termination_instructions = instruction_count(&overflow_code)?;
    let instruction_limit = instruction_count(&machine_code)?;
    Ok(LinkedPush1 {
        assembly,
        machine_code,
        instruction_limit,
        normal_termination_instructions,
        exceptional_termination_instructions,
    })
}

fn normal_exit_assembly() -> String {
    format!(
        "normal_exit:\n\
             \x20   addi t1, s0, -2048\n\
             \x20   addi t1, t1, -2048\n\
             \x20   sd s1, 16(t1)\n\
             \x20   sd zero, 8(t1)\n\
             \x20   addi t0, zero, 1\n\
             \x20   sd t0, 0(t1)\n\
             \x20   jalr zero, -4(t1)\n"
    )
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

fn assembly_error(error: impl fmt::Debug) -> OrchestratorError {
    OrchestratorError::Link {
        description: format!("RV64 assembly failed: {error:?}"),
    }
}

fn instruction_count(machine_code: &[u8]) -> Result<u64, OrchestratorError> {
    if machine_code.len() % RV64_INSTRUCTION_SIZE as usize != 0 {
        return Err(OrchestratorError::Link {
            description: "linked code is not a sequence of 32-bit instructions".to_owned(),
        });
    }
    Ok((machine_code.len() / RV64_INSTRUCTION_SIZE as usize) as u64)
}

fn relative_jal_offset(from_pc: u64, target_pc: u64) -> Result<i64, OrchestratorError> {
    let offset = i128::from(target_pc) - i128::from(from_pc);
    let offset = i64::try_from(offset).map_err(|_| OrchestratorError::Link {
        description: "JAL offset does not fit an i64".to_owned(),
    })?;
    encode_jal_zero(offset)?;
    Ok(offset)
}

/// Encodes `jal zero, offset` after checking RV64 alignment and range.
fn encode_jal_zero(offset: i64) -> Result<u32, OrchestratorError> {
    const MIN_JAL_OFFSET: i64 = -(1 << 20);
    const MAX_JAL_OFFSET: i64 = (1 << 20) - 2;
    if offset & 1 != 0 {
        return Err(OrchestratorError::Link {
            description: format!("JAL offset {offset} is not two-byte aligned"),
        });
    }
    if !(MIN_JAL_OFFSET..=MAX_JAL_OFFSET).contains(&offset) {
        return Err(OrchestratorError::Link {
            description: format!("JAL offset {offset} is outside the signed 21-bit range"),
        });
    }
    let immediate = offset as i32 as u32;
    Ok(((immediate & 0x7fe) << 20) | ((immediate & 0x800) << 9) | (immediate & 0xff000) | 0x6f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_stack_overflow_publishes_an_exception_without_mutating_the_stack() {
        let initial_stack = (0..EVM_STACK_CAPACITY)
            .map(|slot| U256::from(slot + 1))
            .collect::<Vec<_>>();
        let result = execute_guest_push1_with_initial_stack(5, &initial_stack)
            .expect("full-stack PUSH1 reaches the guest exception exit");

        assert_eq!(
            result.guest_result().status(),
            GuestExecutionStatus::GuestException
        );
        assert_eq!(
            result.guest_result().exception_code(),
            PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE
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
        let linked = link_guest_push1(5).expect("link PUSH1");
        assert_eq!(
            assemble(&linked.assembly, Arch::Rv64).expect("reassemble linked source"),
            linked.machine_code
        );
        assert!(!linked.assembly.contains("jal zero, 0"));
    }
}

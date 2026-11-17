//! Minimal Rust orchestration for isolated RV64 guest opcode experiments.
//!
//! This module deliberately links only the code needed to run one `PUSH1`.
//! The opcode body remains owned by [`crate::rv64::push`]; publication and
//! exit code are runtime infrastructure that future isolated opcodes can
//! share without putting termination policy in their generators.

use std::fmt;

use asm_rs::{assemble, Arch};
use revm::primitives::U256;

use crate::{
    compiler::{compile_evm_bytecode, compile_push1_program, CompileError, CompiledRv64Program},
    rv64::{
        add::generate_guest_add,
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

/// Explicitly provisional guest exception code for an EVM binary-operation
/// stack underflow.
pub const PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE: u64 = 2;

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

/// Host-visible result of one linked, isolated guest opcode execution.
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
                write!(f, "could not link isolated guest program: {description}")
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

/// Executes one isolated EVM `ADD` through the RV64 guest ABI.
///
/// `a` occupies the lower stack slot and `b` the top slot, so the initialized
/// guest stack is `[a, b]`. Both the modular arithmetic and the retirement of
/// the top slot are performed by the RV64 guest code.
pub fn execute_guest_add(a: U256, b: U256) -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_add_with_initial_stack(&[a, b])
}

/// Executes the isolated guest `ADD` body with a caller-provided initial stack.
///
/// This exposes the wrapper's guest-level underflow behavior for integration
/// tests without adding ADD to the bytecode compiler or RC execution path.
pub fn execute_guest_add_with_initial_stack(
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    let linked = link_guest_add()?;
    execute_raw_program(
        linked.assembly,
        linked.machine_code,
        linked.normal_termination_instructions,
        linked.exceptional_termination_instructions,
        initial_stack,
    )
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
    execute_raw_program(
        linked.assembly().to_owned(),
        linked.machine_code().to_vec(),
        linked.normal_termination_instructions(),
        linked.exceptional_termination_instructions(),
        initial_stack,
    )
}

fn execute_raw_program(
    assembly: String,
    machine_code: Vec<u8>,
    normal_termination_instructions: u64,
    exceptional_termination_instructions: u64,
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    if initial_stack.len() > EVM_STACK_CAPACITY as usize {
        return Err(OrchestratorError::InitialStackTooDeep {
            depth: initial_stack.len(),
        });
    }

    let mut executor = RvemuExecutor::new()?;
    executor.load_code(&machine_code)?;
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

    let instruction_limit = u64::try_from(machine_code.len() / 4).expect("code length fits u64");
    let report = executor.execute(ENTRY_PC, EXIT_PC, instruction_limit)?;
    let guest_result = read_guest_result(&mut executor, &report)?;
    let guest_termination = match guest_result.status() {
        GuestExecutionStatus::Normal => normal_termination_instructions,
        GuestExecutionStatus::GuestException => exceptional_termination_instructions,
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
        assembly,
        machine_code,
        guest_result,
        instruction_metrics: GuestInstructionMetrics {
            opcode_body,
            guest_termination,
            total: report.executed_instructions,
        },
        executor_identity: EXECUTOR_IDENTITY,
    })
}

struct LinkedGuestAdd {
    assembly: String,
    machine_code: Vec<u8>,
    normal_termination_instructions: u64,
    exceptional_termination_instructions: u64,
}

/// Links the reusable ADD wrapper to its two isolated execution exits.
fn link_guest_add() -> Result<LinkedGuestAdd, OrchestratorError> {
    let generated = generate_guest_add();
    let links = generated
        .link_metadata()
        .ok_or_else(|| OrchestratorError::Link {
            description: "ADD generator did not provide link metadata".to_owned(),
        })?;
    if links.normal_continuation_offset() != generated.machine_code().len() {
        return Err(OrchestratorError::Link {
            description: "ADD normal continuation does not follow its embedded core".to_owned(),
        });
    }
    let core_call = instruction_at(generated.machine_code(), links.core_call_offset())?;
    if core_call & 0x0000_0fff != 0x0000_00ef {
        return Err(OrchestratorError::Link {
            description: "ADD core-call link point is not jal ra".to_owned(),
        });
    }
    if instruction_at(
        generated.machine_code(),
        links.exceptional_trampoline_offset(),
    )? != 0x0000_006f
    {
        return Err(OrchestratorError::Link {
            description: "ADD exceptional link point is not an unpatched jal zero, 0".to_owned(),
        });
    }

    let normal_assembly = add_normal_exit_assembly();
    let underflow_assembly = add_underflow_exit_assembly();
    let normal_code = assemble(&normal_assembly, Arch::Rv64).map_err(assembly_error)?;
    let underflow_code = assemble(&underflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    let normal_start = ENTRY_PC
        + u64::try_from(links.normal_continuation_offset()).expect("ADD code length fits u64");
    let underflow_start =
        normal_start + u64::try_from(normal_code.len()).expect("normal exit length fits u64");
    let trampoline_pc = ENTRY_PC
        + u64::try_from(links.exceptional_trampoline_offset()).expect("ADD link offset fits u64");
    let underflow_offset = relative_jal_offset(trampoline_pc, underflow_start)?;

    let trampoline_source = "guest_add_underflow:\n    jal zero, 0\n";
    let patched_trampoline = format!("guest_add_underflow:\n    jal zero, {underflow_offset}\n");
    let wrapper_assembly = generated
        .assembly()
        .replace(trampoline_source, &patched_trampoline);
    if wrapper_assembly == generated.assembly() {
        return Err(OrchestratorError::Link {
            description: "ADD assembly no longer exposes its underflow trampoline".to_owned(),
        });
    }

    let mut machine_code = generated.machine_code().to_vec();
    let trampoline = encode_jal_zero(underflow_offset)?;
    let trampoline_offset = links.exceptional_trampoline_offset();
    machine_code[trampoline_offset..trampoline_offset + 4]
        .copy_from_slice(&trampoline.to_le_bytes());
    machine_code.extend_from_slice(&normal_code);
    machine_code.extend_from_slice(&underflow_code);
    let assembly = format!("{wrapper_assembly}{normal_assembly}{underflow_assembly}");
    if assemble(&assembly, Arch::Rv64).map_err(assembly_error)? != machine_code {
        return Err(OrchestratorError::Link {
            description: "linked ADD display assembly does not reproduce loaded machine code"
                .to_owned(),
        });
    }
    crate::rv64::memory::validate_generated_code(ENTRY_PC, machine_code.len())?;

    Ok(LinkedGuestAdd {
        assembly,
        machine_code,
        normal_termination_instructions: instruction_count(&normal_code)?,
        exceptional_termination_instructions: instruction_count(&underflow_code)?,
    })
}

fn add_normal_exit_assembly() -> String {
    "guest_add_normal_exit:\n\
        \x20   addi t1, s0, -2048\n\
        \x20   addi t1, t1, -2048\n\
        \x20   sd s1, 16(t1)\n\
        \x20   sd zero, 8(t1)\n\
        \x20   addi t0, zero, 1\n\
        \x20   sd t0, 0(t1)\n\
        \x20   jalr zero, -4(t1)\n"
        .to_owned()
}

fn add_underflow_exit_assembly() -> String {
    format!(
        "guest_add_stack_underflow_exit:\n\
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

fn instruction_at(machine_code: &[u8], offset: usize) -> Result<u32, OrchestratorError> {
    machine_code
        .get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| OrchestratorError::Link {
            description: format!("ADD link offset {offset} is outside generated code"),
        })
}

fn instruction_count(machine_code: &[u8]) -> Result<u64, OrchestratorError> {
    if machine_code.len() % 4 != 0 {
        return Err(OrchestratorError::Link {
            description: "ADD termination code is not instruction aligned".to_owned(),
        });
    }
    Ok(u64::try_from(machine_code.len() / 4).expect("instruction count fits u64"))
}

fn assembly_error(error: impl fmt::Debug) -> OrchestratorError {
    OrchestratorError::Link {
        description: format!("RV64 assembly failed: {error:?}"),
    }
}

fn relative_jal_offset(from_pc: u64, target_pc: u64) -> Result<i64, OrchestratorError> {
    let offset = i64::try_from(i128::from(target_pc) - i128::from(from_pc)).map_err(|_| {
        OrchestratorError::Link {
            description: "ADD JAL offset does not fit an i64".to_owned(),
        }
    })?;
    encode_jal_zero(offset)?;
    Ok(offset)
}

fn encode_jal_zero(offset: i64) -> Result<u32, OrchestratorError> {
    const MIN_JAL_OFFSET: i64 = -(1 << 20);
    const MAX_JAL_OFFSET: i64 = (1 << 20) - 2;
    if offset & 1 != 0 || !(MIN_JAL_OFFSET..=MAX_JAL_OFFSET).contains(&offset) {
        return Err(OrchestratorError::Link {
            description: format!("ADD JAL offset {offset} is not representable"),
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

//! Minimal Rust orchestration for isolated RV64 guest opcode experiments.
//!
//! This module deliberately links only the code needed to run one `PUSH1`.
//! The opcode body remains owned by [`crate::rv64::push`]; publication and
//! exit code are runtime infrastructure that future isolated opcodes can
//! share without putting termination policy in their generators.

use std::fmt;

use alloy_primitives::U256;
#[cfg(test)]
use asm_rs::{assemble, Arch};

#[cfg(test)]
use crate::rv64::{
    codegen::GeneratedRv64,
    opcodes::{add::generate_guest_add, sub::generate_guest_sub},
    verify::{verify_final_image, Fragment, PublisherLayout},
};
use crate::{
    compiler::{compile_evm_bytecode, CompileError, CompiledRv64Program},
    rv64::{
        executor::{ExecutorError, IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{
            initialize_guest_state, read_guest_result, write_evm_word, GuestInitializationError,
            GuestMemoryError, GuestResult, GuestResultError,
        },
        memory::{LayoutError, ENTRY_PC, EVM_STACK_CAPACITY, EVM_STACK_REGION, EXIT_PC},
    },
};

/// rvemu version used by this initial isolated-opcode orchestrator.
pub const EXECUTOR_IDENTITY: &str = "rvemu 0.0.11";

pub use crate::compiler::PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE;

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
    execute_guest_evm_bytecode(&[0x60, immediate])
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
    let linked = compile_evm_bytecode(&[0x01]).map_err(map_compilation_error)?;
    execute_raw_program(
        linked.assembly().to_owned(),
        linked.machine_code().to_vec(),
        linked.termination_entry_pcs(),
        initial_stack,
    )
}

/// Executes one isolated EVM `SUB` through the RV64 guest ABI.
///
/// `a` occupies the lower stack slot and `b` the top slot, so the initialized
/// guest stack is `[a, b]`. The RV64 guest evaluates `b - a` modulo 2^256.
pub fn execute_guest_sub(a: U256, b: U256) -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_sub_with_initial_stack(&[a, b])
}

/// Executes the isolated guest `SUB` body with a caller-provided initial stack.
///
/// This is test-facing ABI setup that exposes the wrapper's guest-level
/// underflow behavior without adding SUB to bytecode compilation or RC.
pub fn execute_guest_sub_with_initial_stack(
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    let linked = compile_evm_bytecode(&[0x03]).map_err(map_compilation_error)?;
    execute_raw_program(
        linked.assembly().to_owned(),
        linked.machine_code().to_vec(),
        linked.termination_entry_pcs(),
        initial_stack,
    )
}

fn map_compilation_error(error: CompileError) -> OrchestratorError {
    match error {
        CompileError::Layout(error) => OrchestratorError::Layout(error),
        CompileError::Link { description } => OrchestratorError::Link { description },
        error => OrchestratorError::Compile(error),
    }
}

/// Executes one isolated EVM `POP` through the shared compiled RV64 body.
pub fn execute_guest_pop() -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_pop_with_initial_stack(&[U256::ZERO])
}

/// Executes the isolated guest `POP` body with caller-provided stack words.
///
/// This uses RC's linked POP body directly, so RO does not maintain a Rust
/// implementation of the opcode or a duplicate termination path.
pub fn execute_guest_pop_with_initial_stack(
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    execute_linked_program(compile_evm_bytecode(&[0x50])?, initial_stack)
}

/// Executes one isolated EVM `DUP1` through the shared compiled RV64 body.
pub fn execute_guest_dup1() -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_dup1_with_initial_stack(&[U256::ZERO])
}

/// Executes the isolated guest `DUP1` body with a caller-provided stack.
///
/// The duplication itself is performed entirely by generated RV64 code.
pub fn execute_guest_dup1_with_initial_stack(
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    execute_linked_program(compile_evm_bytecode(&[0x80])?, initial_stack)
}

/// Decodes, compiles, and executes a supported EVM bytecode program once.
///
/// The executor runs the resulting RV64 executable as a whole; Rust does not
/// execute individual EVM operations.
pub fn execute_guest_evm_bytecode(
    bytecode: &[u8],
) -> Result<OrchestratorResult, OrchestratorError> {
    execute_guest_evm_bytecode_with_initial_stack(bytecode, &[])
}

/// Executes supported bytecode once with a host-provided valid initial stack.
///
/// This is test-facing ABI setup; compilation itself never interprets EVM
/// operations. It also exposes ADD's zero- and one-word underflow paths.
pub fn execute_guest_evm_bytecode_with_initial_stack(
    bytecode: &[u8],
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    execute_linked_program(compile_evm_bytecode(bytecode)?, initial_stack)
}

fn execute_linked_program(
    linked: CompiledRv64Program,
    initial_stack: &[U256],
) -> Result<OrchestratorResult, OrchestratorError> {
    execute_raw_program(
        linked.assembly().to_owned(),
        linked.machine_code().to_vec(),
        linked.termination_entry_pcs(),
        initial_stack,
    )
}

fn execute_raw_program(
    assembly: String,
    machine_code: Vec<u8>,
    termination_entry_pcs: &[u64],
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
    // Stop before the dynamically selected publisher, then execute that
    // publisher to the sentinel. Both component counts are observed by rvemu.
    let body_report = executor.execute_until(ENTRY_PC, termination_entry_pcs, instruction_limit)?;
    let termination_limit = instruction_limit
        .checked_sub(body_report.executed_instructions)
        .ok_or(OrchestratorError::ExecutionAccounting {
            total: body_report.executed_instructions,
            termination: 0,
        })?;
    let termination_report = executor.execute(body_report.exit_pc, EXIT_PC, termination_limit)?;
    let guest_result = read_guest_result(&mut executor, &termination_report)?;
    let total = body_report
        .executed_instructions
        .checked_add(termination_report.executed_instructions)
        .ok_or(OrchestratorError::ExecutionAccounting {
            total: u64::MAX,
            termination: termination_report.executed_instructions,
        })?;

    Ok(OrchestratorResult {
        assembly,
        machine_code,
        guest_result,
        instruction_metrics: GuestInstructionMetrics {
            opcode_body: body_report.executed_instructions,
            guest_termination: termination_report.executed_instructions,
            total,
        },
        executor_identity: EXECUTOR_IDENTITY,
    })
}

#[cfg(test)]
struct LinkedGuestArithmetic {
    assembly: String,
    machine_code: Vec<u8>,
    termination_entry_pcs: Vec<u64>,
}

/// Links the reusable ADD wrapper to its two isolated execution exits.
#[cfg(test)]
fn link_guest_add() -> Result<LinkedGuestArithmetic, OrchestratorError> {
    link_guest_arithmetic(
        generate_guest_add(),
        "ADD",
        "guest_add_underflow",
        "guest_add_normal_exit",
        "guest_add_stack_underflow_exit",
    )
}

/// Links the reusable SUB wrapper to the same isolated execution exits as ADD.
#[cfg(test)]
fn link_guest_sub() -> Result<LinkedGuestArithmetic, OrchestratorError> {
    link_guest_arithmetic(
        generate_guest_sub(),
        "SUB",
        "guest_sub_underflow",
        "guest_sub_normal_exit",
        "guest_sub_stack_underflow_exit",
    )
}

/// Links a binary arithmetic wrapper to normal and stack-underflow publishers.
#[cfg(test)]
fn link_guest_arithmetic(
    generated: GeneratedRv64,
    operation: &str,
    underflow_label: &str,
    normal_exit_label: &str,
    underflow_exit_label: &str,
) -> Result<LinkedGuestArithmetic, OrchestratorError> {
    let links = generated
        .link_metadata()
        .ok_or_else(|| OrchestratorError::Link {
            description: format!("{operation} generator did not provide link metadata"),
        })?;
    if links.normal_continuation_offset() != generated.machine_code().len() {
        return Err(OrchestratorError::Link {
            description: format!(
                "{operation} normal continuation does not follow its embedded core"
            ),
        });
    }
    let core_call = instruction_at(generated.machine_code(), links.core_call_offset())?;
    if core_call & 0x0000_0fff != 0x0000_00ef {
        return Err(OrchestratorError::Link {
            description: format!("{operation} core-call link point is not jal ra"),
        });
    }
    if instruction_at(
        generated.machine_code(),
        links.exceptional_trampoline_offset(),
    )? != 0x0000_006f
    {
        return Err(OrchestratorError::Link {
            description: format!(
                "{operation} exceptional link point is not an unpatched jal zero, 0"
            ),
        });
    }

    let normal_assembly = arithmetic_normal_exit_assembly(normal_exit_label);
    let underflow_assembly = arithmetic_underflow_exit_assembly(underflow_exit_label);
    let normal_code = assemble(&normal_assembly, Arch::Rv64).map_err(assembly_error)?;
    let underflow_code = assemble(&underflow_assembly, Arch::Rv64).map_err(assembly_error)?;
    let normal_start =
        ENTRY_PC + u64::try_from(links.normal_continuation_offset()).expect("code length fits u64");
    let underflow_start =
        normal_start + u64::try_from(normal_code.len()).expect("normal exit length fits u64");
    let trampoline_pc = ENTRY_PC
        + u64::try_from(links.exceptional_trampoline_offset()).expect("link offset fits u64");
    let underflow_offset = relative_jal_offset(trampoline_pc, underflow_start)?;

    let trampoline_source = format!("{underflow_label}:\n    jal zero, 0\n");
    let patched_trampoline = format!("{underflow_label}:\n    jal zero, {underflow_offset}\n");
    let wrapper_assembly = generated
        .assembly()
        .replace(&trampoline_source, &patched_trampoline);
    if wrapper_assembly == generated.assembly() {
        return Err(OrchestratorError::Link {
            description: format!("{operation} assembly no longer exposes its underflow trampoline"),
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
            description: format!(
                "linked {operation} display assembly does not reproduce loaded machine code"
            ),
        });
    }
    crate::rv64::memory::validate_generated_code(ENTRY_PC, machine_code.len())?;
    let termination_entry_pcs = vec![normal_start, underflow_start];
    verify_final_image(
        &machine_code,
        [Fragment::Arithmetic],
        PublisherLayout::IsolatedArithmetic,
        &termination_entry_pcs,
    )
    .map_err(|error| OrchestratorError::Link {
        description: format!("RV64 final-image verification failed: {error}"),
    })?;

    Ok(LinkedGuestArithmetic {
        assembly,
        machine_code,
        termination_entry_pcs,
    })
}

#[cfg(test)]
fn arithmetic_normal_exit_assembly(label: &str) -> String {
    format!(
        "{label}:\n\
        \x20   addi t1, s0, -2048\n\
        \x20   addi t1, t1, -2048\n\
        \x20   sd s1, 16(t1)\n\
        \x20   sd zero, 8(t1)\n\
        \x20   addi t0, zero, 1\n\
        \x20   sd t0, 0(t1)\n\
        \x20   jalr zero, -4(t1)\n"
    )
}

#[cfg(test)]
fn arithmetic_underflow_exit_assembly(label: &str) -> String {
    format!(
        "{label}:\n\
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

#[cfg(test)]
fn instruction_at(machine_code: &[u8], offset: usize) -> Result<u32, OrchestratorError> {
    machine_code
        .get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| OrchestratorError::Link {
            description: format!("link offset {offset} is outside generated code"),
        })
}

#[cfg(test)]
fn assembly_error(error: impl fmt::Debug) -> OrchestratorError {
    OrchestratorError::Link {
        description: format!("RV64 assembly failed: {error:?}"),
    }
}

#[cfg(test)]
fn relative_jal_offset(from_pc: u64, target_pc: u64) -> Result<i64, OrchestratorError> {
    let offset = i64::try_from(i128::from(target_pc) - i128::from(from_pc)).map_err(|_| {
        OrchestratorError::Link {
            description: "JAL offset does not fit an i64".to_owned(),
        }
    })?;
    encode_jal_zero(offset)?;
    Ok(offset)
}

#[cfg(test)]
fn encode_jal_zero(offset: i64) -> Result<u32, OrchestratorError> {
    const MIN_JAL_OFFSET: i64 = -(1 << 20);
    const MAX_JAL_OFFSET: i64 = (1 << 20) - 2;
    if offset & 1 != 0 || !(MIN_JAL_OFFSET..=MAX_JAL_OFFSET).contains(&offset) {
        return Err(OrchestratorError::Link {
            description: format!("JAL offset {offset} is not representable"),
        });
    }
    let immediate = offset as i32 as u32;
    Ok(((immediate & 0x7fe) << 20) | ((immediate & 0x800) << 9) | (immediate & 0xff000) | 0x6f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rv64::guest::GuestExecutionStatus;

    #[test]
    fn full_stack_overflow_publishes_an_exception_without_mutating_the_stack() {
        let initial_stack = (0..EVM_STACK_CAPACITY)
            .map(|slot| U256::from(slot + 1))
            .collect::<Vec<_>>();
        let result = execute_guest_evm_bytecode_with_initial_stack(&[0x60, 5], &initial_stack)
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
            let result = execute_guest_evm_bytecode(&[0x60, immediate])
                .expect("PUSH1 completes through RV64");

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
    fn isolated_arithmetic_images_match_legacy_singleton_linkers() {
        for (opcode, execute, link_legacy) in [
            (
                0x01,
                execute_guest_add as fn(U256, U256) -> _,
                link_guest_add as fn() -> _,
            ),
            (0x03, execute_guest_sub, link_guest_sub),
        ] {
            let legacy = link_legacy().unwrap();
            let compiled = compile_evm_bytecode(&[opcode]).unwrap();
            let isolated = execute(U256::from(2), U256::from(3)).unwrap();

            assert_eq!(
                compiled.machine_code(),
                legacy.machine_code,
                "opcode {opcode:#04x}"
            );
            assert_eq!(
                compiled.termination_entry_pcs(),
                legacy.termination_entry_pcs
            );
            assert_eq!(
                compiled.termination_entry_pcs(),
                [ENTRY_PC + 172, ENTRY_PC + 200]
            );
            assert_ne!(compiled.assembly(), legacy.assembly);
            assert_eq!(isolated.machine_code(), compiled.machine_code());
            assert_eq!(isolated.assembly(), compiled.assembly());
        }
    }

    #[test]
    fn linked_assembly_reassembles_to_the_loaded_machine_code() {
        let linked = compile_evm_bytecode(&[0x60, 5]).expect("link PUSH1");
        assert_eq!(
            asm_rs::assemble(linked.assembly(), asm_rs::Arch::Rv64)
                .expect("reassemble linked source"),
            linked.machine_code()
        );
        assert!(!linked.assembly().contains("jal zero, 0"));
    }
}

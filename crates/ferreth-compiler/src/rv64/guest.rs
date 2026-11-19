//! Backend-neutral definitions for the published RV64 guest result record.
//!
//! The record begins at [`crate::rv64::memory::METADATA_REGION.start()`].
//! This module deliberately does not repeat that address: the memory layout is
//! its single source of truth.  A terminal record is valid only after the host
//! executor has independently observed its configured deliberate exit
//! boundary; status bytes alone never establish that fact.

use std::fmt;

use alloy_primitives::U256;

use super::{
    executor::{ExecutionReport, ExecutorError, IntegerRegister, Rv64Executor},
    memory::{
        evm_stack_depth, validate_evm_stack_layout, LayoutError, EVM_STACK_CAPACITY,
        EVM_STACK_REGION, EVM_WORD_SIZE, METADATA_REGION,
    },
};

/// Offset of the `status` field in the guest result record.
pub const GUEST_METADATA_STATUS_OFFSET: usize = 0x00;
/// Offset of the `exception_code` field in the guest result record.
pub const GUEST_METADATA_EXCEPTION_CODE_OFFSET: usize = 0x08;
/// Offset of the `final_s1` field in the guest result record.
pub const GUEST_METADATA_FINAL_S1_OFFSET: usize = 0x10;
/// Exact encoded size of the guest result record.
pub const GUEST_METADATA_ENCODED_SIZE: usize = 24;

/// Exact byte width of one EVM word in guest memory.
pub const EVM_WORD_ENCODED_SIZE: usize = EVM_WORD_SIZE as usize;

/// Side-effect-free failures of an EVM operand-stack operation precondition.
///
/// These describe intentional guest-level stack errors, not executor failures
/// or corrupted ABI state.  The current helpers accept a next-free pointer and
/// derive its depth through [`evm_stack_depth`], so pointer bounds, alignment,
/// and the architectural capacity all have one source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackError {
    InvalidStackPointer(LayoutError),
    StackOverflow { depth: u64 },
    StackUnderflow { depth: u64, required: u64 },
}

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStackPointer(error) => {
                write!(f, "invalid EVM stack pointer for operation: {error}")
            }
            Self::StackOverflow { depth } => write!(
                f,
                "EVM stack push at depth {depth} exceeds capacity {EVM_STACK_CAPACITY}"
            ),
            Self::StackUnderflow { depth, required } => write!(
                f,
                "EVM stack operation requires {required} live words, but depth is {depth}"
            ),
        }
    }
}

impl std::error::Error for StackError {}

impl From<LayoutError> for StackError {
    fn from(error: LayoutError) -> Self {
        Self::InvalidStackPointer(error)
    }
}

/// Validates that an EVM push can use the current next-free stack pointer.
///
/// This only checks the operation precondition. It never writes a word or
/// updates `s1`.
pub fn validate_evm_stack_push(next_free: u64) -> Result<(), StackError> {
    let depth = evm_stack_depth(next_free)?;
    if depth == EVM_STACK_CAPACITY {
        Err(StackError::StackOverflow { depth })
    } else {
        Ok(())
    }
}

/// Validates that an EVM pop can use the current next-free stack pointer.
///
/// This only checks the operation precondition. It never reads a word or
/// updates `s1`.
pub fn validate_evm_stack_pop(next_free: u64) -> Result<(), StackError> {
    validate_evm_stack_operands(next_free, 1)
}

/// Validates that an EVM operation has the required number of live operands.
///
/// This is suitable for binary operations with `required == 2`. It is pure:
/// invalid operations leave guest registers, metadata, and stack memory
/// untouched because this helper accepts only copied scalar values.
pub fn validate_evm_stack_operands(next_free: u64, required: u64) -> Result<(), StackError> {
    let depth = evm_stack_depth(next_free)?;
    if depth < required {
        Err(StackError::StackUnderflow { depth, required })
    } else {
        Ok(())
    }
}

/// Failures while reading or writing an EVM word in the guest stack region.
///
/// These host-side helpers only access the fixed stack reservation. They do
/// not determine whether a slot is live, and never modify guest registers or
/// metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestMemoryError {
    InvalidStackSlot { slot_index: usize },
    Layout(LayoutError),
    Executor(ExecutorError),
}

impl fmt::Display for GuestMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStackSlot { slot_index } => write!(
                f,
                "EVM stack slot {slot_index} is outside the valid range 0..{EVM_STACK_CAPACITY}"
            ),
            Self::Layout(error) => write!(f, "invalid EVM stack word layout: {error}"),
            Self::Executor(error) => write!(f, "could not access EVM stack word: {error}"),
        }
    }
}

impl std::error::Error for GuestMemoryError {}

impl From<LayoutError> for GuestMemoryError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<ExecutorError> for GuestMemoryError {
    fn from(error: ExecutorError) -> Self {
        Self::Executor(error)
    }
}

/// Writes one complete EVM word to a fixed guest stack slot.
///
/// Slot zero begins at [`EVM_STACK_REGION`]'s base; slot 1023 is the final
/// valid slot. The four 64-bit limbs are written in least-significant-limb
/// first order, with each limb explicitly encoded little-endian. This does
/// not adjust `s0`, `s1`, or metadata, so it is suitable only for host-side
/// setup and inspection rather than guest stack operations.
pub fn write_evm_word(
    executor: &mut impl Rv64Executor,
    slot_index: usize,
    word: U256,
) -> Result<(), GuestMemoryError> {
    let address = evm_stack_slot_address(slot_index)?;
    executor.write_memory(address, &encode_evm_word(word))?;
    Ok(())
}

/// Reads one complete EVM word from a fixed guest stack slot.
///
/// This is the inverse of [`write_evm_word`]. It does not inspect or modify
/// guest registers or metadata, and does not establish stack liveness.
pub fn read_evm_word(
    executor: &mut impl Rv64Executor,
    slot_index: usize,
) -> Result<U256, GuestMemoryError> {
    let address = evm_stack_slot_address(slot_index)?;
    let mut bytes = [0; EVM_WORD_ENCODED_SIZE];
    executor.read_memory(address, &mut bytes)?;
    Ok(decode_evm_word(bytes))
}

fn evm_stack_slot_address(slot_index: usize) -> Result<u64, GuestMemoryError> {
    validate_evm_stack_layout()?;
    if slot_index >= EVM_STACK_CAPACITY as usize {
        return Err(GuestMemoryError::InvalidStackSlot { slot_index });
    }

    let slot_index =
        u64::try_from(slot_index).map_err(|_| GuestMemoryError::InvalidStackSlot { slot_index })?;
    let slot_offset =
        slot_index
            .checked_mul(EVM_WORD_SIZE)
            .ok_or(LayoutError::AddressOverflow {
                address: EVM_STACK_REGION.start(),
                width: EVM_WORD_SIZE,
            })?;
    let address =
        EVM_STACK_REGION
            .start()
            .checked_add(slot_offset)
            .ok_or(LayoutError::AddressOverflow {
                address: EVM_STACK_REGION.start(),
                width: slot_offset,
            })?;
    EVM_STACK_REGION.validate_aligned_access(address, EVM_WORD_ENCODED_SIZE, EVM_WORD_SIZE)?;
    Ok(address)
}

fn encode_evm_word(word: U256) -> [u8; EVM_WORD_ENCODED_SIZE] {
    let big_endian = word.to_be_bytes::<EVM_WORD_ENCODED_SIZE>();
    let mut encoded = [0; EVM_WORD_ENCODED_SIZE];
    for limb_index in 0..EVM_WORD_ENCODED_SIZE / std::mem::size_of::<u64>() {
        let big_endian_start = EVM_WORD_ENCODED_SIZE - (limb_index + 1) * 8;
        let limb = u64::from_be_bytes(
            big_endian[big_endian_start..big_endian_start + 8]
                .try_into()
                .expect("EVM word limbs are always eight bytes"),
        );
        let little_endian_start = limb_index * 8;
        encoded[little_endian_start..little_endian_start + 8].copy_from_slice(&limb.to_le_bytes());
    }
    encoded
}

fn decode_evm_word(encoded: [u8; EVM_WORD_ENCODED_SIZE]) -> U256 {
    let mut big_endian = [0; EVM_WORD_ENCODED_SIZE];
    for limb_index in 0..EVM_WORD_ENCODED_SIZE / std::mem::size_of::<u64>() {
        let little_endian_start = limb_index * 8;
        let limb = u64::from_le_bytes(
            encoded[little_endian_start..little_endian_start + 8]
                .try_into()
                .expect("EVM word limbs are always eight bytes"),
        );
        let big_endian_start = EVM_WORD_ENCODED_SIZE - (limb_index + 1) * 8;
        big_endian[big_endian_start..big_endian_start + 8].copy_from_slice(&limb.to_be_bytes());
    }
    U256::from_be_bytes(big_endian)
}

/// Failures while preparing an executor for a new guest execution.
///
/// A failed initialization never reports a ready guest. In particular, an
/// executor memory-write failure is returned to the caller rather than being
/// represented as guest metadata. No rollback of a partially completed backend
/// memory write is attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestInitializationError {
    Layout(LayoutError),
    Executor(ExecutorError),
}

impl fmt::Display for GuestInitializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "invalid guest initialization layout: {error}"),
            Self::Executor(error) => write!(f, "could not initialize guest state: {error}"),
        }
    }
}

impl std::error::Error for GuestInitializationError {}

impl From<LayoutError> for GuestInitializationError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<ExecutorError> for GuestInitializationError {
    fn from(error: ExecutorError) -> Self {
        Self::Executor(error)
    }
}

/// Initializes the ABI-owned state for one new EVM guest execution.
///
/// This uses only [`Rv64Executor`]: it sets `s0` and `s1` to the EVM stack
/// base and writes the initial 24-byte `RUNNING` metadata record. Therefore
/// the stack is logically empty even when a reused executor still contains
/// stale bytes in the stack reservation; no stack word is live while
/// `s0 == s1`, so the initializer neither clears nor reads those bytes.
///
/// The current guest code does not use the native call stack, so `sp` and all
/// unrelated registers are preserved. A later runtime that requires `sp` must
/// initialize it independently inside [`super::memory::NATIVE_STACK_REGION`]
/// with the required ABI alignment. Code loading and execution-PC management
/// also remain the caller's responsibilities: use [`super::memory::ENTRY_PC`]
/// and [`super::memory::EXIT_PC`] with [`Rv64Executor::execute`].
///
/// Repeated calls restore the two ABI registers and overwrite all 24 metadata
/// bytes. The metadata write occurs before either register is changed, so a
/// reported write failure does not present a newly initialized register pair as
/// successful initialization.
pub fn initialize_guest_state(
    executor: &mut impl Rv64Executor,
) -> Result<(), GuestInitializationError> {
    validate_evm_stack_layout()?;
    METADATA_REGION.validate_aligned_access(
        METADATA_REGION.start(),
        GUEST_METADATA_ENCODED_SIZE,
        std::mem::size_of::<u64>() as u64,
    )?;

    let stack_base = EVM_STACK_REGION.start();
    let metadata = GuestMetadata::new(GuestExecutionStatus::Running, 0, stack_base).encode();
    executor.write_memory(METADATA_REGION.start(), &metadata)?;
    executor.write_register(IntegerRegister::EVM_STACK_BASE, stack_base);
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, stack_base);
    Ok(())
}

/// Backend-neutral guest execution status published in the result record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum GuestExecutionStatus {
    /// Execution is in progress, or no complete result has been published.
    Running = 0,
    /// Guest execution completed normally.
    Normal = 1,
    /// Guest execution completed with a guest-published exception.
    GuestException = 2,
}

impl GuestExecutionStatus {
    /// Returns the stable `u64` ABI value for this status.
    pub const fn as_u64(self) -> u64 {
        self as u64
    }

    /// Whether this status can represent a published terminal result.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Normal | Self::GuestException)
    }
}

/// A decoded guest result record independent of executor implementation.
///
/// Rust layout is intentionally irrelevant: [`Self::encode`] and
/// [`Self::decode`] always use the fixed little-endian ABI representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestMetadata {
    status: GuestExecutionStatus,
    exception_code: u64,
    final_s1: u64,
}

impl GuestMetadata {
    pub const fn new(status: GuestExecutionStatus, exception_code: u64, final_s1: u64) -> Self {
        Self {
            status,
            exception_code,
            final_s1,
        }
    }

    pub const fn status(self) -> GuestExecutionStatus {
        self.status
    }

    pub const fn exception_code(self) -> u64 {
        self.exception_code
    }

    pub const fn final_s1(self) -> u64 {
        self.final_s1
    }

    /// Encodes exactly one ABI record using explicit little-endian fields.
    pub fn encode(self) -> [u8; GUEST_METADATA_ENCODED_SIZE] {
        let mut bytes = [0; GUEST_METADATA_ENCODED_SIZE];
        bytes[GUEST_METADATA_STATUS_OFFSET..GUEST_METADATA_EXCEPTION_CODE_OFFSET]
            .copy_from_slice(&self.status.as_u64().to_le_bytes());
        bytes[GUEST_METADATA_EXCEPTION_CODE_OFFSET..GUEST_METADATA_FINAL_S1_OFFSET]
            .copy_from_slice(&self.exception_code.to_le_bytes());
        bytes[GUEST_METADATA_FINAL_S1_OFFSET..GUEST_METADATA_ENCODED_SIZE]
            .copy_from_slice(&self.final_s1.to_le_bytes());
        bytes
    }

    /// Decodes exactly one ABI record using explicit little-endian fields.
    ///
    /// This establishes structural validity: the encoded size is exact and
    /// the status is one of the ABI's defined numeric values.  It does not
    /// establish that the record is a published terminal result.
    pub fn decode(bytes: &[u8]) -> Result<Self, GuestMetadataError> {
        if bytes.len() != GUEST_METADATA_ENCODED_SIZE {
            return Err(GuestMetadataError::InvalidEncodedSize {
                actual: bytes.len(),
            });
        }

        let status = GuestExecutionStatus::try_from(read_u64(bytes, GUEST_METADATA_STATUS_OFFSET))?;
        Ok(Self::new(
            status,
            read_u64(bytes, GUEST_METADATA_EXCEPTION_CODE_OFFSET),
            read_u64(bytes, GUEST_METADATA_FINAL_S1_OFFSET),
        ))
    }

    /// Validates a result record after the executor independently verified
    /// deliberate arrival at its configured exit boundary.
    ///
    /// Callers must pass `true` only for that executor-observed condition;
    /// this method never infers exit from metadata contents.  A `RUNNING`
    /// record is incomplete even if its other fields look terminal.
    pub fn validate_published_terminal(
        self,
        exit_boundary_verified: bool,
    ) -> Result<PublishedGuestMetadata, GuestMetadataError> {
        if !self.status.is_terminal() {
            return Err(GuestMetadataError::StillRunningOrIncomplete);
        }
        if self.status == GuestExecutionStatus::Normal && self.exception_code != 0 {
            return Err(GuestMetadataError::NormalStatusHasExceptionCode {
                exception_code: self.exception_code,
            });
        }
        let stack_depth =
            evm_stack_depth(self.final_s1).map_err(GuestMetadataError::InvalidFinalStackPointer)?;
        if !exit_boundary_verified {
            return Err(GuestMetadataError::ExitBoundaryNotVerified);
        }
        Ok(PublishedGuestMetadata {
            metadata: self,
            stack_depth,
        })
    }
}

impl TryFrom<u64> for GuestExecutionStatus {
    type Error = GuestMetadataError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Running),
            1 => Ok(Self::Normal),
            2 => Ok(Self::GuestException),
            _ => Err(GuestMetadataError::InvalidStatus { value }),
        }
    }
}

/// A terminal guest result that passed metadata and stack validation.
///
/// Its existence also records that the caller independently verified the
/// executor's deliberate exit boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedGuestMetadata {
    metadata: GuestMetadata,
    stack_depth: u64,
}

impl PublishedGuestMetadata {
    pub const fn metadata(self) -> GuestMetadata {
        self.metadata
    }

    pub const fn status(self) -> GuestExecutionStatus {
        self.metadata.status()
    }

    pub const fn exception_code(self) -> u64 {
        self.metadata.exception_code()
    }

    pub const fn final_s1(self) -> u64 {
        self.metadata.final_s1()
    }

    /// Depth derived from the ABI-validated final next-free stack pointer.
    pub const fn stack_depth(self) -> u64 {
        self.stack_depth
    }
}

/// A host-retrieved terminal EVM guest result.
///
/// `stack` is ordered from slot zero through the top item, so the top of a
/// nonempty EVM stack is its last element.  The result deliberately retains
/// the guest-published status and exception code: a guest exception is a
/// successfully retrieved guest result, not an executor failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestResult {
    metadata: PublishedGuestMetadata,
    stack: Vec<U256>,
}

impl GuestResult {
    pub const fn metadata(&self) -> PublishedGuestMetadata {
        self.metadata
    }

    pub const fn status(&self) -> GuestExecutionStatus {
        self.metadata.status()
    }

    pub const fn exception_code(&self) -> u64 {
        self.metadata.exception_code()
    }

    pub const fn final_s1(&self) -> u64 {
        self.metadata.final_s1()
    }

    pub const fn stack_depth(&self) -> u64 {
        self.metadata.stack_depth()
    }

    pub fn stack(&self) -> &[U256] {
        &self.stack
    }
}

/// Failures while retrieving a guest-published terminal result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestResultError {
    Layout(LayoutError),
    Executor(ExecutorError),
    Metadata(GuestMetadataError),
    LiveStackBaseMismatch {
        expected_s0: u64,
        live_s0: u64,
    },
    LiveStackPointerMismatch {
        published_s1: u64,
        live_s1: u64,
    },
    StackWord {
        slot_index: usize,
        error: GuestMemoryError,
    },
}

impl fmt::Display for GuestResultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "invalid guest result layout: {error}"),
            Self::Executor(error) => write!(f, "could not read guest result metadata: {error}"),
            Self::Metadata(error) => write!(f, "invalid guest result metadata: {error}"),
            Self::LiveStackBaseMismatch {
                expected_s0,
                live_s0,
            } => write!(
                f,
                "live s0 0x{live_s0:016x} does not match the EVM stack base 0x{expected_s0:016x}"
            ),
            Self::LiveStackPointerMismatch {
                published_s1,
                live_s1,
            } => write!(
                f,
                "published final s1 0x{published_s1:016x} does not match live s1 0x{live_s1:016x}"
            ),
            Self::StackWord { slot_index, error } => {
                write!(
                    f,
                    "could not read live EVM stack slot {slot_index}: {error}"
                )
            }
        }
    }
}

impl std::error::Error for GuestResultError {}

impl From<LayoutError> for GuestResultError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<ExecutorError> for GuestResultError {
    fn from(error: ExecutorError) -> Self {
        Self::Executor(error)
    }
}

impl From<GuestMetadataError> for GuestResultError {
    fn from(error: GuestMetadataError) -> Self {
        Self::Metadata(error)
    }
}

/// Reads and validates a terminal EVM guest result after a successful run.
///
/// The [`Rv64Executor::execute`] contract makes a returned
/// [`ExecutionReport`] evidence that its configured exit PC was observed
/// before another instruction was fetched.  Thus this function neither treats
/// metadata as termination proof nor introduces a duplicate completion flag.
/// Instruction-limit exhaustion and backend faults yield `ExecutorError`
/// instead of an `ExecutionReport`, and therefore cannot be passed here from a
/// failed execution.
pub fn read_guest_result(
    executor: &mut impl Rv64Executor,
    execution_report: &ExecutionReport,
) -> Result<GuestResult, GuestResultError> {
    // A report can only be obtained from the successful return path of
    // `Rv64Executor::execute`; see that trait method's contract above.
    let _ = execution_report;

    METADATA_REGION.validate_aligned_access(
        METADATA_REGION.start(),
        GUEST_METADATA_ENCODED_SIZE,
        std::mem::size_of::<u64>() as u64,
    )?;
    let mut encoded_metadata = [0; GUEST_METADATA_ENCODED_SIZE];
    executor.read_memory(METADATA_REGION.start(), &mut encoded_metadata)?;
    let metadata = GuestMetadata::decode(&encoded_metadata)?;
    let published_metadata = metadata.validate_published_terminal(true)?;

    let expected_s0 = EVM_STACK_REGION.start();
    let live_s0 = executor.read_register(IntegerRegister::EVM_STACK_BASE);
    if live_s0 != expected_s0 {
        return Err(GuestResultError::LiveStackBaseMismatch {
            expected_s0,
            live_s0,
        });
    }

    let live_s1 = executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE);
    if live_s1 != published_metadata.final_s1() {
        return Err(GuestResultError::LiveStackPointerMismatch {
            published_s1: published_metadata.final_s1(),
            live_s1,
        });
    }

    let stack_depth = usize::try_from(published_metadata.stack_depth())
        .expect("the ABI-bounded EVM stack depth always fits usize");
    let mut stack = Vec::with_capacity(stack_depth);
    for slot_index in 0..stack_depth {
        stack.push(
            read_evm_word(executor, slot_index)
                .map_err(|error| GuestResultError::StackWord { slot_index, error })?,
        );
    }

    Ok(GuestResult {
        metadata: published_metadata,
        stack,
    })
}

/// Failures while decoding or validating a guest result record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestMetadataError {
    InvalidEncodedSize { actual: usize },
    InvalidStatus { value: u64 },
    StillRunningOrIncomplete,
    InvalidFinalStackPointer(LayoutError),
    NormalStatusHasExceptionCode { exception_code: u64 },
    ExitBoundaryNotVerified,
}

impl fmt::Display for GuestMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEncodedSize { actual } => write!(
                f,
                "guest metadata has {actual} bytes; expected {GUEST_METADATA_ENCODED_SIZE}"
            ),
            Self::InvalidStatus { value } => {
                write!(f, "guest metadata has unknown status value {value}")
            }
            Self::StillRunningOrIncomplete => {
                write!(f, "guest metadata is still running or incomplete")
            }
            Self::InvalidFinalStackPointer(error) => {
                write!(f, "guest metadata has invalid final s1: {error}")
            }
            Self::NormalStatusHasExceptionCode { exception_code } => write!(
                f,
                "normal guest metadata must have exception code zero, got {exception_code}"
            ),
            Self::ExitBoundaryNotVerified => write!(
                f,
                "guest metadata cannot be accepted before the executor verifies the exit boundary"
            ),
        }
    }
}

impl std::error::Error for GuestMetadataError {}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let end = offset + std::mem::size_of::<u64>();
    u64::from_le_bytes(
        bytes[offset..end]
            .try_into()
            .expect("metadata length was checked before field decoding"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rv64::memory::{EVM_STACK_CAPACITY, EVM_STACK_REGION};

    #[test]
    fn record_has_exact_abi_length_and_little_endian_offsets() {
        let metadata = GuestMetadata::new(
            GuestExecutionStatus::GuestException,
            0x1122_3344_5566_7788,
            0x99aa_bbcc_ddee_ff00,
        );
        let encoded = metadata.encode();

        assert_eq!(GUEST_METADATA_ENCODED_SIZE, 24);
        assert_eq!(encoded.len(), GUEST_METADATA_ENCODED_SIZE);
        assert_eq!(
            &encoded[GUEST_METADATA_STATUS_OFFSET..0x08],
            &2_u64.to_le_bytes()
        );
        assert_eq!(
            &encoded[GUEST_METADATA_EXCEPTION_CODE_OFFSET..0x10],
            &0x1122_3344_5566_7788_u64.to_le_bytes()
        );
        assert_eq!(
            &encoded[GUEST_METADATA_FINAL_S1_OFFSET..],
            &0x99aa_bbcc_ddee_ff00_u64.to_le_bytes()
        );
    }

    #[test]
    fn encoding_and_decoding_round_trip() {
        let metadata = GuestMetadata::new(
            GuestExecutionStatus::GuestException,
            42,
            EVM_STACK_REGION.start() + 64,
        );
        assert_eq!(GuestMetadata::decode(&metadata.encode()), Ok(metadata));
    }

    #[test]
    fn execution_status_values_match_the_abi() {
        assert_eq!(GuestExecutionStatus::Running.as_u64(), 0);
        assert_eq!(GuestExecutionStatus::Normal.as_u64(), 1);
        assert_eq!(GuestExecutionStatus::GuestException.as_u64(), 2);
        assert_eq!(
            GuestExecutionStatus::try_from(0),
            Ok(GuestExecutionStatus::Running)
        );
        assert_eq!(
            GuestExecutionStatus::try_from(1),
            Ok(GuestExecutionStatus::Normal)
        );
        assert_eq!(
            GuestExecutionStatus::try_from(2),
            Ok(GuestExecutionStatus::GuestException)
        );
    }

    #[test]
    fn decoding_rejects_unknown_status_and_wrong_record_size() {
        let mut encoded =
            GuestMetadata::new(GuestExecutionStatus::Running, 0, EVM_STACK_REGION.start()).encode();
        encoded[GUEST_METADATA_STATUS_OFFSET..0x08].copy_from_slice(&3_u64.to_le_bytes());
        assert_eq!(
            GuestMetadata::decode(&encoded),
            Err(GuestMetadataError::InvalidStatus { value: 3 })
        );
        assert_eq!(
            GuestMetadata::decode(&encoded[..GUEST_METADATA_ENCODED_SIZE - 1]),
            Err(GuestMetadataError::InvalidEncodedSize {
                actual: GUEST_METADATA_ENCODED_SIZE - 1
            })
        );
    }

    #[test]
    fn terminal_validation_rejects_invalid_stack_pointer() {
        let metadata = GuestMetadata::new(
            GuestExecutionStatus::Normal,
            0,
            EVM_STACK_REGION.start() + 1,
        );
        assert!(matches!(
            metadata.validate_published_terminal(true),
            Err(GuestMetadataError::InvalidFinalStackPointer(
                LayoutError::StackPointerMisaligned { .. }
            ))
        ));
    }

    #[test]
    fn normal_terminal_validation_requires_zero_exception_code() {
        let metadata =
            GuestMetadata::new(GuestExecutionStatus::Normal, 1, EVM_STACK_REGION.start());
        assert_eq!(
            metadata.validate_published_terminal(true),
            Err(GuestMetadataError::NormalStatusHasExceptionCode { exception_code: 1 })
        );
    }

    #[test]
    fn running_metadata_is_not_a_published_terminal_result() {
        let metadata =
            GuestMetadata::new(GuestExecutionStatus::Running, 0, EVM_STACK_REGION.start());
        assert_eq!(
            metadata.validate_published_terminal(true),
            Err(GuestMetadataError::StillRunningOrIncomplete)
        );
    }

    #[test]
    fn terminal_validation_requires_an_executor_verified_exit_boundary() {
        let metadata =
            GuestMetadata::new(GuestExecutionStatus::Normal, 0, EVM_STACK_REGION.start());
        assert_eq!(
            metadata.validate_published_terminal(false),
            Err(GuestMetadataError::ExitBoundaryNotVerified)
        );
    }

    #[test]
    fn empty_and_full_stack_terminal_records_are_valid() {
        let empty = GuestMetadata::new(GuestExecutionStatus::Normal, 0, EVM_STACK_REGION.start())
            .validate_published_terminal(true)
            .expect("empty stack record must be valid");
        let full = GuestMetadata::new(
            GuestExecutionStatus::GuestException,
            17,
            EVM_STACK_REGION.end(),
        )
        .validate_published_terminal(true)
        .expect("full stack record must be valid");

        assert_eq!(empty.stack_depth(), 0);
        assert_eq!(full.stack_depth(), EVM_STACK_CAPACITY);
    }

    #[test]
    fn stack_preconditions_enforce_boundaries_without_mutating_state() {
        let empty = EVM_STACK_REGION.start();
        let one_word = empty + EVM_WORD_SIZE;
        let two_words = empty + 2 * EVM_WORD_SIZE;
        let almost_full = EVM_STACK_REGION.end() - EVM_WORD_SIZE;
        let full = EVM_STACK_REGION.end();
        let invalid_depth = full + EVM_WORD_SIZE;

        assert_eq!(validate_evm_stack_push(empty), Ok(()));
        assert_eq!(validate_evm_stack_push(almost_full), Ok(()));
        assert_eq!(
            validate_evm_stack_push(full),
            Err(StackError::StackOverflow {
                depth: EVM_STACK_CAPACITY
            })
        );

        assert_eq!(
            validate_evm_stack_pop(empty),
            Err(StackError::StackUnderflow {
                depth: 0,
                required: 1
            })
        );
        assert_eq!(validate_evm_stack_pop(one_word), Ok(()));

        for next_free in [empty, one_word] {
            assert_eq!(
                validate_evm_stack_operands(next_free, 2),
                Err(StackError::StackUnderflow {
                    depth: evm_stack_depth(next_free).expect("test stack pointer is valid"),
                    required: 2,
                })
            );
        }
        assert_eq!(validate_evm_stack_operands(two_words, 2), Ok(()));
        assert!(matches!(
            validate_evm_stack_push(invalid_depth),
            Err(StackError::InvalidStackPointer(
                LayoutError::StackPointerOutOfRange { .. }
            ))
        ));

        // The helpers have no executor or mutable arguments, so rejected
        // checks cannot change logical guest state.
        let registers = [empty, full];
        let metadata = GuestMetadata::new(GuestExecutionStatus::Running, 0, empty);
        let stack_memory = [0xa5_u8; EVM_WORD_ENCODED_SIZE];
        let _ = validate_evm_stack_push(full);
        let _ = validate_evm_stack_pop(empty);
        let _ = validate_evm_stack_operands(one_word, 2);
        assert_eq!(registers, [empty, full]);
        assert_eq!(
            metadata,
            GuestMetadata::new(GuestExecutionStatus::Running, 0, empty)
        );
        assert_eq!(stack_memory, [0xa5_u8; EVM_WORD_ENCODED_SIZE]);
    }
}

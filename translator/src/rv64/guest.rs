//! Backend-neutral definitions for the published RV64 guest result record.
//!
//! The record begins at [`crate::rv64::memory::METADATA_REGION.start()`].
//! This module deliberately does not repeat that address: the memory layout is
//! its single source of truth.  A terminal record is valid only after the host
//! executor has independently observed its configured deliberate exit
//! boundary; status bytes alone never establish that fact.

use std::fmt;

use super::{
    executor::{ExecutorError, IntegerRegister, Rv64Executor},
    memory::{
        evm_stack_depth, validate_evm_stack_layout, LayoutError, EVM_STACK_REGION, METADATA_REGION,
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
}

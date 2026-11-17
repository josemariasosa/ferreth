//! Initial rvemu guest memory layout and pure validation helpers.
//!
//! These are logical guest-ABI regions.  They do not replace an executor's
//! physical-memory validation: an address accepted by an executor's DRAM
//! bounds is not automatically valid for metadata or either stack region.

use std::fmt;

/// The width of an instruction emitted by the current RV64 generator.
///
/// Compressed instructions are not part of the current generation model.
pub const RV64_INSTRUCTION_SIZE: u64 = 4;

/// The entry address for generated RV64 code in the initial rvemu layout.
pub const ENTRY_PC: u64 = 0x8000_0000;
/// The deliberately reserved exit sentinel PC.
pub const EXIT_PC: u64 = 0x800f_fffc;

/// A half-open logical guest-memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    start: u64,
    end: u64,
}

impl MemoryRegion {
    /// Creates a half-open region `[start, end)`.
    pub const fn new(start: u64, end: u64) -> Self {
        assert!(start <= end, "memory region end precedes start");
        Self { start, end }
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end
    }

    pub const fn size(self) -> u64 {
        self.end - self.start
    }

    /// Returns whether `address` identifies a byte in this region.
    pub const fn contains(self, address: u64) -> bool {
        self.start <= address && address < self.end
    }

    /// Validates an access whose entire byte range must be in this region.
    ///
    /// A zero-length access is accepted only at an address in the inclusive
    /// interval `[start, end]`; accepting `end` matches half-open range
    /// semantics while still rejecting unrelated addresses.
    pub fn validate_access(self, address: u64, width: usize) -> Result<(), LayoutError> {
        if width == 0 {
            return if self.start <= address && address <= self.end {
                Ok(())
            } else {
                Err(LayoutError::AccessOutsideRegion {
                    region: self,
                    address,
                    width,
                })
            };
        }

        let width = u64::try_from(width).expect("usize fits in u64 on supported hosts");
        let end = address
            .checked_add(width)
            .ok_or(LayoutError::AddressOverflow { address, width })?;
        if self.start <= address && end <= self.end {
            Ok(())
        } else {
            Err(LayoutError::AccessOutsideRegion {
                region: self,
                address,
                width: width as usize,
            })
        }
    }

    /// Validates both the byte range and an ABI-required alignment.
    pub fn validate_aligned_access(
        self,
        address: u64,
        width: usize,
        alignment: u64,
    ) -> Result<(), LayoutError> {
        validate_alignment(address, alignment)?;
        self.validate_access(address, width)
    }
}

/// Generated code reservation, including the dedicated exit slot.
pub const CODE_REGION: MemoryRegion = MemoryRegion::new(0x8000_0000, 0x8010_0000);
/// Guest metadata and result-memory reservation.
pub const METADATA_REGION: MemoryRegion = MemoryRegion::new(0x8010_0000, 0x8010_1000);
/// EVM operand-stack reservation.
pub const EVM_STACK_REGION: MemoryRegion = MemoryRegion::new(0x8010_1000, 0x8010_9000);
/// Address space intentionally not assigned to initial guest state.
pub const UNASSIGNED_REGION: MemoryRegion = MemoryRegion::new(0x8010_9000, 0xbfff_0000);
/// Native RV64 call-stack reservation, independent of the EVM operand stack.
pub const NATIVE_STACK_REGION: MemoryRegion = MemoryRegion::new(0xbfff_0000, 0xc000_0000);

pub const EVM_WORD_SIZE: u64 = 32;
pub const EVM_STACK_CAPACITY: u64 = 1024;
pub const EVM_STACK_SIZE: u64 = EVM_WORD_SIZE * EVM_STACK_CAPACITY;

/// A named logical part of the initial guest layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestRegion {
    Code,
    Metadata,
    EvmStack,
    NativeStack,
    Unassigned,
}

impl GuestRegion {
    pub const fn memory_region(self) -> MemoryRegion {
        match self {
            Self::Code => CODE_REGION,
            Self::Metadata => METADATA_REGION,
            Self::EvmStack => EVM_STACK_REGION,
            Self::NativeStack => NATIVE_STACK_REGION,
            Self::Unassigned => UNASSIGNED_REGION,
        }
    }
}

/// Errors returned by the initial logical guest-layout validators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    AddressOverflow {
        address: u64,
        width: u64,
    },
    AccessOutsideRegion {
        region: MemoryRegion,
        address: u64,
        width: usize,
    },
    InvalidAlignment {
        alignment: u64,
    },
    MisalignedAddress {
        address: u64,
        alignment: u64,
    },
    InvalidCodeEntry {
        entry_pc: u64,
    },
    EmptyCode,
    CodeLengthNotInstructionAligned {
        length: u64,
    },
    CodeOverlapsExitSlot {
        code_end: u64,
    },
    InvalidStackLayout,
    StackPointerOutOfRange {
        next_free: u64,
    },
    StackPointerMisaligned {
        next_free: u64,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddressOverflow { address, width } => write!(
                f,
                "guest memory address 0x{address:016x} overflows with access width {width}"
            ),
            Self::AccessOutsideRegion {
                region,
                address,
                width,
            } => write!(
                f,
                "guest memory access [0x{address:016x}, 0x{:016x}) with width {width} is outside logical region [0x{:016x}, 0x{:016x})",
                address.saturating_add(*width as u64),
                region.start,
                region.end,
            ),
            Self::InvalidAlignment { alignment } => {
                write!(f, "guest alignment {alignment} is not a nonzero power of two")
            }
            Self::MisalignedAddress { address, alignment } => write!(
                f,
                "guest address 0x{address:016x} is not aligned to {alignment} bytes"
            ),
            Self::InvalidCodeEntry { entry_pc } => write!(
                f,
                "generated code entry 0x{entry_pc:016x} is not the layout entry PC 0x{ENTRY_PC:016x}"
            ),
            Self::EmptyCode => write!(f, "generated RV64 code must not be empty"),
            Self::CodeLengthNotInstructionAligned { length } => write!(
                f,
                "generated RV64 code length {length} is not divisible by {RV64_INSTRUCTION_SIZE}"
            ),
            Self::CodeOverlapsExitSlot { code_end } => write!(
                f,
                "generated RV64 code ending at 0x{code_end:016x} reaches the reserved exit slot at 0x{EXIT_PC:016x}"
            ),
            Self::InvalidStackLayout => write!(f, "the fixed EVM stack layout violates its ABI invariants"),
            Self::StackPointerOutOfRange { next_free } => write!(
                f,
                "EVM next-free stack pointer 0x{next_free:016x} is outside the inclusive stack range"
            ),
            Self::StackPointerMisaligned { next_free } => write!(
                f,
                "EVM next-free stack pointer 0x{next_free:016x} is not word aligned from the stack base"
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

/// Validates an ABI alignment requirement and an address against it.
pub fn validate_alignment(address: u64, alignment: u64) -> Result<(), LayoutError> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(LayoutError::InvalidAlignment { alignment });
    }
    if address.is_multiple_of(alignment) {
        Ok(())
    } else {
        Err(LayoutError::MisalignedAddress { address, alignment })
    }
}

/// Validates generated code loaded at `entry_pc` against the code reservation.
///
/// The generated code uses only base 32-bit RV64 instructions, begins at the
/// fixed entry PC, and must end no later than the first byte of the dedicated
/// exit slot.  Consequently it can never execute or overwrite that slot.
pub fn validate_generated_code(entry_pc: u64, code_len: usize) -> Result<(), LayoutError> {
    if entry_pc != ENTRY_PC {
        return Err(LayoutError::InvalidCodeEntry { entry_pc });
    }
    if code_len == 0 {
        return Err(LayoutError::EmptyCode);
    }
    let code_len = u64::try_from(code_len).expect("usize fits in u64 on supported hosts");
    if code_len % RV64_INSTRUCTION_SIZE != 0 {
        return Err(LayoutError::CodeLengthNotInstructionAligned { length: code_len });
    }
    let code_end = entry_pc
        .checked_add(code_len)
        .ok_or(LayoutError::AddressOverflow {
            address: entry_pc,
            width: code_len,
        })?;
    if code_end > EXIT_PC {
        return Err(LayoutError::CodeOverlapsExitSlot { code_end });
    }
    Ok(())
}

/// Checks the fixed EVM stack constants against the accepted ABI.
pub fn validate_evm_stack_layout() -> Result<(), LayoutError> {
    if !EVM_STACK_REGION.start().is_multiple_of(EVM_WORD_SIZE)
        || EVM_STACK_REGION.size() != EVM_STACK_SIZE
        || EVM_STACK_SIZE != 32 * 1024
        || EVM_STACK_CAPACITY != 1024
    {
        return Err(LayoutError::InvalidStackLayout);
    }
    Ok(())
}

/// Derives the EVM stack depth from its next-free pointer.
///
/// Both the empty pointer (the stack base) and the full pointer (the exclusive
/// stack end) are valid.  No stack mutation is performed here.
pub fn evm_stack_depth(next_free: u64) -> Result<u64, LayoutError> {
    validate_evm_stack_layout()?;
    if next_free < EVM_STACK_REGION.start() || next_free > EVM_STACK_REGION.end() {
        return Err(LayoutError::StackPointerOutOfRange { next_free });
    }
    let offset = next_free - EVM_STACK_REGION.start();
    if !offset.is_multiple_of(EVM_WORD_SIZE) {
        return Err(LayoutError::StackPointerMisaligned { next_free });
    }
    Ok(offset / EVM_WORD_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_sizes_and_boundaries_match_the_initial_layout() {
        assert_eq!(CODE_REGION.size(), 0x10_0000);
        assert_eq!(METADATA_REGION.size(), 0x1000);
        assert_eq!(EVM_STACK_REGION.size(), EVM_STACK_SIZE);
        assert_eq!(NATIVE_STACK_REGION.size(), 0x1_0000);
        assert_eq!(CODE_REGION.end(), METADATA_REGION.start());
        assert_eq!(METADATA_REGION.end(), EVM_STACK_REGION.start());
        assert_eq!(EVM_STACK_REGION.end(), UNASSIGNED_REGION.start());
        assert_eq!(UNASSIGNED_REGION.end(), NATIVE_STACK_REGION.start());
        assert_eq!(EXIT_PC + RV64_INSTRUCTION_SIZE, CODE_REGION.end());
    }

    #[test]
    fn code_can_end_immediately_before_the_exit_slot() {
        assert_eq!(
            validate_generated_code(ENTRY_PC, (EXIT_PC - ENTRY_PC) as usize),
            Ok(())
        );
    }

    #[test]
    fn code_validation_rejects_the_exit_slot_oversize_empty_and_bad_alignment() {
        assert!(matches!(
            validate_generated_code(ENTRY_PC, (EXIT_PC - ENTRY_PC + 4) as usize),
            Err(LayoutError::CodeOverlapsExitSlot { .. })
        ));
        assert_eq!(
            validate_generated_code(ENTRY_PC, 0),
            Err(LayoutError::EmptyCode)
        );
        assert!(matches!(
            validate_generated_code(ENTRY_PC, 6),
            Err(LayoutError::CodeLengthNotInstructionAligned { length: 6 })
        ));
        assert_eq!(
            validate_generated_code(ENTRY_PC + 4, 4),
            Err(LayoutError::InvalidCodeEntry {
                entry_pc: ENTRY_PC + 4
            })
        );
    }

    #[test]
    fn logical_accesses_require_the_entire_range_and_required_alignment() {
        assert_eq!(
            METADATA_REGION.validate_access(METADATA_REGION.end() - 1, 1),
            Ok(())
        );
        assert!(matches!(
            METADATA_REGION.validate_access(METADATA_REGION.end() - 1, 2),
            Err(LayoutError::AccessOutsideRegion { .. })
        ));
        assert!(matches!(
            EVM_STACK_REGION.validate_aligned_access(EVM_STACK_REGION.start() + 8, 32, 32),
            Err(LayoutError::MisalignedAddress { .. })
        ));
    }

    #[test]
    fn logical_access_rejects_address_overflow() {
        assert_eq!(
            METADATA_REGION.validate_access(u64::MAX, 2),
            Err(LayoutError::AddressOverflow {
                address: u64::MAX,
                width: 2
            })
        );
    }

    #[test]
    fn zero_length_accesses_are_limited_to_the_closed_region_boundary() {
        assert_eq!(
            METADATA_REGION.validate_access(METADATA_REGION.start(), 0),
            Ok(())
        );
        assert_eq!(
            METADATA_REGION.validate_access(METADATA_REGION.end(), 0),
            Ok(())
        );
        assert!(matches!(
            METADATA_REGION.validate_access(METADATA_REGION.end() + 1, 0),
            Err(LayoutError::AccessOutsideRegion { .. })
        ));
    }

    #[test]
    fn stack_depth_accepts_empty_and_full_stack() {
        assert_eq!(evm_stack_depth(EVM_STACK_REGION.start()), Ok(0));
        assert_eq!(
            evm_stack_depth(EVM_STACK_REGION.end()),
            Ok(EVM_STACK_CAPACITY)
        );
    }

    #[test]
    fn stack_depth_rejects_misaligned_and_out_of_range_pointers() {
        assert!(matches!(
            evm_stack_depth(EVM_STACK_REGION.start() + 1),
            Err(LayoutError::StackPointerMisaligned { .. })
        ));
        assert!(matches!(
            evm_stack_depth(EVM_STACK_REGION.start() - 1),
            Err(LayoutError::StackPointerOutOfRange { .. })
        ));
        assert!(matches!(
            evm_stack_depth(EVM_STACK_REGION.end() + 1),
            Err(LayoutError::StackPointerOutOfRange { .. })
        ));
    }
}

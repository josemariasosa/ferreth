//! Reusable RV64 code generation for EVM `PUSH1`.
//!
//! The emitted sequence operates on the guest ABI's `s0`/`s1` stack pair.  Its
//! final instruction is an explicit, patchable overflow trampoline: it loops
//! until a later runtime links it to the guest-exception publisher.  A
//! successful push jumps over that trampoline and falls through into the next
//! generated guest operation.

/// Generated RV64 code and the assembly from which its bytes were assembled.
///
/// Keeping the display assembly and machine code together makes inspection
/// possible without maintaining a second hand-encoded instruction stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedRv64 {
    pub(crate) assembly: String,
    pub(crate) machine_code: Vec<u8>,
    pub(crate) link_metadata: Option<GeneratedRv64LinkMetadata>,
}

/// Named offsets for an opcode body which embeds a callable RV64 core.
///
/// Offsets are relative to the start of [`GeneratedRv64::machine_code`]. The
/// normal continuation is the first byte after the embedded core, where a
/// later composed guest program may place its next opcode body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratedRv64LinkMetadata {
    pub(crate) core_call_offset: usize,
    pub(crate) normal_continuation_offset: usize,
    pub(crate) exceptional_trampoline_offset: usize,
}

impl GeneratedRv64LinkMetadata {
    pub const fn core_call_offset(self) -> usize {
        self.core_call_offset
    }

    pub const fn normal_continuation_offset(self) -> usize {
        self.normal_continuation_offset
    }

    pub const fn exceptional_trampoline_offset(self) -> usize {
        self.exceptional_trampoline_offset
    }
}

impl GeneratedRv64 {
    /// Readable RV64 assembly for the generated operation.
    pub fn assembly(&self) -> &str {
        &self.assembly
    }

    /// Little-endian bytes of consecutive 32-bit RV64 instructions.
    pub fn machine_code(&self) -> &[u8] {
        &self.machine_code
    }

    /// Machine-code instruction words in address order.
    pub fn instruction_words(&self) -> impl Iterator<Item = u32> + '_ {
        self.machine_code
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes))
    }

    /// Optional link information for an opcode body with an embedded core.
    pub const fn link_metadata(&self) -> Option<GeneratedRv64LinkMetadata> {
        self.link_metadata
    }

    /// Byte offset of the patchable overflow trampoline instruction.
    ///
    /// The instruction is `jal zero, 0`, initially a self-jump. A later guest
    /// runtime can replace it with a transfer to its exception publication
    /// path without changing the preceding PUSH1 body.
    pub fn overflow_trampoline_offset(&self) -> usize {
        self.machine_code.len() - 4
    }
}

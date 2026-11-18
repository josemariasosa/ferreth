//! Reusable stack-aware RV64 implementation of EVM `ADD`.
//!
//! The wrapper embeds the established pointer-based `evm_add` leaf core. Its
//! `jal ra, evm_add` gives that core a return address at the `s1` update, so
//! the host exit sentinel in `ra` is never involved in this guest-local call.

use asm_rs::{assemble, Arch};

use crate::rv64::codegen::{GeneratedRv64, GeneratedRv64LinkMetadata};
use crate::Opcode;

/// Generates one stack-aware RV64 implementation of EVM `ADD`.
///
/// The generated wrapper verifies that `s1 >= s0 + 64` before calculating
/// operand pointers. On success it calls the existing `evm_add` core with
/// `a0 = s1 - 32`, `a1 = a2 = s1 - 64`, then retires the inactive top slot by
/// decrementing `s1`. Underflow enters a patchable `jal zero, 0` trampoline
/// without accessing stack memory or changing guest stack registers.
pub fn generate_guest_add() -> GeneratedRv64 {
    // Keep the arithmetic implementation in one place: this is the exact
    // assembly produced by the historical ADD generator, embedded as a leaf
    // subroutine rather than reimplemented here.
    let add_core = Opcode::Add
        .assembly()
        .lines()
        // asm-rs accepts the core's instructions and label but not its ELF
        // display directives. This is the same filtering used by the
        // historical execution harness.
        .filter(|line| !line.trim_start().starts_with('.'))
        .collect::<Vec<_>>()
        .join("\n");
    let core_machine_code = assemble(&add_core, Arch::Rv64)
        .expect("existing ADD RV64 assembly must assemble into base RV64 instructions");
    // The jump is at byte 28; it skips the underflow trampoline and the core
    // to the one-past-the-end continuation, which a later composition can
    // make the entry of its next opcode body.
    let normal_jump_offset = core_machine_code.len() + 8;
    let assembly = format!(
        "guest_add:\n\
             \x20   addi t0, s0, 64\n\
             \x20   bltu s1, t0, guest_add_underflow\n\
             \x20   addi a0, s1, -32\n\
             \x20   addi a1, s1, -64\n\
             \x20   addi a2, s1, -64\n\
             \x20   jal ra, evm_add\n\
             \x20   addi s1, s1, -32\n\
             \x20   jal zero, {normal_jump_offset}\n\
             guest_add_underflow:\n\
             \x20   jal zero, 0\n\
             {add_core}\n"
    );
    let machine_code = assemble(&assembly, Arch::Rv64)
        .expect("fixed guest ADD RV64 assembly must assemble into base RV64 instructions");

    // The wrapper before `evm_add` is nine fixed-width instructions. The sixth
    // is the call; the ninth is the unresolved underflow trampoline.
    const CORE_START_OFFSET: usize = 9 * 4;
    const CORE_CALL_OFFSET: usize = 5 * 4;
    const UNDERFLOW_TRAMPOLINE_OFFSET: usize = 8 * 4;
    let normal_continuation_offset = CORE_START_OFFSET + core_machine_code.len();
    assert_eq!(machine_code.len(), normal_continuation_offset);

    GeneratedRv64 {
        assembly,
        machine_code,
        link_metadata: Some(GeneratedRv64LinkMetadata {
            core_call_offset: CORE_CALL_OFFSET,
            normal_continuation_offset,
            exceptional_trampoline_offset: UNDERFLOW_TRAMPOLINE_OFFSET,
        }),
    }
}

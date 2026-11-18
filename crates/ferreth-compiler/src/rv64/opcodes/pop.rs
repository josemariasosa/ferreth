//! Reusable stack-aware RV64 implementation of EVM `POP`.
//!
//! POP only changes the guest next-free stack pointer. Its underflow path is
//! an initially self-jumping trampoline, patched by the program linker to the
//! shared guest stack-underflow publisher.

use asm_rs::{assemble, Arch};

use crate::rv64::codegen::GeneratedRv64;

/// Generates one stack-aware RV64 implementation of EVM `POP`.
///
/// The generated code checks that `s1 > s0` before retiring the top word.
/// Success decrements `s1` by exactly one EVM word without reading or writing
/// stack memory; failure leaves both ABI stack registers unchanged and enters
/// a patchable `jal zero, 0` underflow trampoline.
pub fn generate_guest_pop() -> GeneratedRv64 {
    let assembly = [
        "guest_pop:\n",
        "    bgeu s0, s1, guest_pop_underflow\n",
        "    addi s1, s1, -32\n",
        "    jal zero, 8\n",
        "guest_pop_underflow:\n",
        "    jal zero, 0\n",
    ]
    .concat();
    let machine_code = assemble(&assembly, Arch::Rv64)
        .expect("fixed POP RV64 assembly must assemble into base RV64 instructions");
    assert_eq!(machine_code.len(), 4 * 4, "POP encoding changed width");

    GeneratedRv64 {
        assembly,
        machine_code,
        link_metadata: None,
    }
}

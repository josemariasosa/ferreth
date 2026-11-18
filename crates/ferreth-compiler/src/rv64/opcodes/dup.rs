//! Reusable stack-aware RV64 implementation of EVM `DUP1`.
//!
//! The body has separate patchable underflow and overflow trampolines. A
//! successful execution copies the top U256 word to the next-free slot and
//! advances `s1`.

use asm_rs::{assemble, Arch};

use crate::rv64::codegen::GeneratedRv64;

/// Generates one stack-aware RV64 implementation of EVM `DUP1`.
///
/// The generated code checks underflow and capacity before touching stack
/// memory. It copies all four little-endian limbs, preserves `s0` and the
/// original top word, and increments `s1` only after all stores complete.
pub fn generate_guest_dup1() -> GeneratedRv64 {
    let assembly = [
        "guest_dup1:\n",
        "    bgeu s0, s1, guest_dup1_underflow\n",
        "    lui t0, 8\n",
        "    add t0, s0, t0\n",
        "    bgeu s1, t0, guest_dup1_overflow\n",
        "    ld t1, -32(s1)\n",
        "    ld t2, -24(s1)\n",
        "    ld t3, -16(s1)\n",
        "    ld t4, -8(s1)\n",
        "    sd t1, 0(s1)\n",
        "    sd t2, 8(s1)\n",
        "    sd t3, 16(s1)\n",
        "    sd t4, 24(s1)\n",
        "    addi s1, s1, 32\n",
        "    jal zero, 12\n",
        "guest_dup1_underflow:\n",
        "    jal zero, 0\n",
        "guest_dup1_overflow:\n",
        "    jal zero, 0\n",
    ]
    .concat();
    let machine_code = assemble(&assembly, Arch::Rv64)
        .expect("fixed DUP1 RV64 assembly must assemble into base RV64 instructions");
    assert_eq!(machine_code.len(), 16 * 4, "DUP1 encoding changed width");

    GeneratedRv64 {
        assembly,
        machine_code,
        link_metadata: None,
    }
}

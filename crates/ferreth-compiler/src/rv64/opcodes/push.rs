use std::fmt::Write;

use asm_rs::{assemble, Arch};

use crate::rv64::codegen::GeneratedRv64;

/// Generates one stack-aware RV64 implementation of EVM `PUSH1`.
///
/// `s1` is checked against the ABI stack limit before any stack store or stack
/// pointer update.  On success, it writes the immediate in limb zero, clears
/// the remaining three limbs, and advances `s1` by one 32-byte EVM word.  It
/// neither reads nor writes `sp`, and it preserves `s0`.
pub fn generate_guest_push1(immediate: u8) -> GeneratedRv64 {
    let mut assembly = String::from("guest_push1:\n");
    emit_push1_body(&mut assembly, immediate, "32", "8");
    assembly.push_str("    jal zero, 0\n");
    let machine_code = assemble(&assembly, Arch::Rv64)
        .expect("fixed PUSH1 RV64 assembly must assemble into base RV64 instructions");
    assert_eq!(machine_code.len(), 11 * 4, "PUSH1 encoding changed width");

    GeneratedRv64 {
        assembly,
        machine_code,
        link_metadata: None,
    }
}

pub(crate) fn emit_symbolic_push1(
    assembly: &mut String,
    immediate: u8,
    index: usize,
    overflow_publisher: &str,
) {
    let overflow = format!("push1_overflow_{index}");
    let continuation = format!("push1_continue_{index}");
    writeln!(assembly, "guest_push1_{index}:").unwrap();
    emit_push1_body(assembly, immediate, &overflow, &continuation);
    writeln!(
        assembly,
        "{overflow}:\n    jal zero, {overflow_publisher}\n{continuation}:"
    )
    .unwrap();
}

fn emit_push1_body(assembly: &mut String, immediate: u8, overflow: &str, continuation: &str) {
    // Deriving the limit from s0 avoids materializing the 0x8010_9000 guest
    // address, whose 32-bit sign extension is inconvenient on RV64.
    writeln!(
        assembly,
        "    lui t0, 8\n\
             \x20   add t0, s0, t0\n\
             \x20   bgeu s1, t0, {overflow}\n\
             \x20   addi t0, zero, {immediate}\n\
             \x20   sd t0, 0(s1)\n\
             \x20   sd zero, 8(s1)\n\
             \x20   sd zero, 16(s1)\n\
             \x20   sd zero, 24(s1)\n\
             \x20   addi s1, s1, 32\n\
             \x20   jal zero, {continuation}"
    )
    .unwrap();
}

use std::fmt::Write;

#[derive(Debug, PartialEq, Eq)]
pub enum Opcode {
    Add,
}

impl Opcode {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::Add),
            _ => None,
        }
    }

    pub fn assembly(&self) -> String {
        match self {
            Self::Add => generate_add(),
        }
    }
}

/// Adds little-endian limbs modulo 2^256.
pub fn add_u256(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut result = [0; 4];
    let mut carry = false;
    for i in 0..4 {
        let (sum, carry_a) = a[i].overflowing_add(b[i]);
        let (sum, carry_b) = sum.overflowing_add(u64::from(carry));
        result[i] = sum;
        carry = carry_a || carry_b;
    }
    result
}

fn generate_add() -> String {
    let mut asm = String::from(
        ".text\n\
         .p2align 2\n\
         .globl evm_add\n\
         .type evm_add, @function\n\
         evm_add:\n\
         \x20   li t3, 0\n",
    );
    for offset in [0, 8, 16, 24] {
        writeln!(
            asm,
            "    ld t0, {offset}(a0)\n\
             \x20   ld t1, {offset}(a1)\n\
             \x20   add t2, t0, t1\n\
             \x20   sltu t4, t2, t0\n\
             \x20   add t2, t2, t3\n\
             \x20   sltu t5, t2, t3\n\
             \x20   or t3, t4, t5\n\
             \x20   sd t2, {offset}(a2)"
        )
        .expect("writing to a String cannot fail");
    }
    asm.push_str("    ret\n.size evm_add, .-evm_add\n");
    asm
}

#[cfg(test)]
mod tests {
    use super::{add_u256, Opcode};

    #[test]
    fn identifies_add_opcode() {
        assert_eq!(Opcode::from_byte(0x01), Some(Opcode::Add));
        assert_eq!(Opcode::from_byte(0x00), None);
    }

    #[test]
    fn zero_operands() {
        assert_eq!(add_u256([0; 4], [0; 4]), [0; 4]);
    }

    #[test]
    fn simple_addition() {
        assert_eq!(add_u256([5, 0, 0, 0], [7, 0, 0, 0]), [12, 0, 0, 0]);
    }

    #[test]
    fn carry_across_limb_boundary() {
        assert_eq!(add_u256([u64::MAX, 0, 0, 0], [1, 0, 0, 0]), [0, 1, 0, 0]);
    }

    #[test]
    fn carry_propagates_through_all_limbs() {
        assert_eq!(
            add_u256([u64::MAX, u64::MAX, u64::MAX, 0], [1, 0, 0, 0]),
            [0, 0, 0, 1]
        );
    }

    #[test]
    fn overflow_wraps_to_zero() {
        assert_eq!(add_u256([u64::MAX; 4], [1, 0, 0, 0]), [0; 4]);
    }

    #[test]
    fn randomized_against_u128_limb_arithmetic() {
        let mut state = 0x1234_5678_9abc_def0_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..10_000 {
            let a = std::array::from_fn(|_| next());
            let b = std::array::from_fn(|_| next());
            let mut expected = [0; 4];
            let mut carry = 0_u128;
            for i in 0..4 {
                let sum = u128::from(a[i]) + u128::from(b[i]) + carry;
                expected[i] = sum as u64;
                carry = sum >> 64;
            }
            assert_eq!(add_u256(a, b), expected, "a={a:?}, b={b:?}");
        }
    }
}

use std::fmt::Write;

use super::{
    PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE, PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
};
use crate::rv64::guest::GuestExecutionStatus;

#[derive(Clone, Copy)]
pub(super) enum Publisher {
    Normal,
    Overflow,
    Underflow,
}

impl Publisher {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal_exit",
            Self::Overflow => "push1_stack_overflow_exit",
            Self::Underflow => "binary_stack_underflow_exit",
        }
    }

    pub(super) fn assembly(self) -> String {
        let label = self.label();
        let mut source = format!(
            "{label}:\n\
             \x20   addi t1, s0, -2048\n\
             \x20   addi t1, t1, -2048\n\
             \x20   sd s1, 16(t1)\n"
        );
        let code = match self {
            Self::Normal => None,
            Self::Overflow => Some(PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE),
            Self::Underflow => Some(PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE),
        };
        let status = if let Some(code) = code {
            writeln!(source, "    addi t0, zero, {code}\n    sd t0, 8(t1)").unwrap();
            GuestExecutionStatus::GuestException
        } else {
            source.push_str("    sd zero, 8(t1)\n");
            GuestExecutionStatus::Normal
        };
        let status = status.as_u64();
        writeln!(
            source,
            "    addi t0, zero, {status}\n    sd t0, 0(t1)\n    jalr zero, -4(t1)"
        )
        .unwrap();
        source
    }
}

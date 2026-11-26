//! RV64 opcode generation.
//!
//! The public `generate_guest_*` functions expose historical unlinked opcode
//! bodies for inspection and regression tests. They have no frame gas
//! accounting or terminal publisher. Use [`crate::compiler::compile_evm_bytecode`]
//! to produce a complete metered image and
//! [`crate::orchestrator::execute_guest_evm_bytecode`] to execute it.

pub mod add;
mod arithmetic;
pub mod dup;
pub(crate) mod gas;
pub mod pop;
pub mod push;
pub mod sub;

use rvemu::{bus::DRAM_BASE, emulator::Emulator};

use super::{ExecutionReport, ExecutorError, IntegerRegister, Rv64Executor};

// rvemu 0.0.11 allocates one GiB of DRAM beginning at DRAM_BASE. Keep this
// adapter-local fact behind the backend-neutral executor interface.
const DRAM_SIZE: u64 = 1024 * 1024 * 1024;
const DRAM_END: u64 = DRAM_BASE + DRAM_SIZE;

/// Executor implementation backed by rvemu 0.0.11.
pub struct RvemuExecutor {
    emulator: Emulator,
}

impl RvemuExecutor {
    pub const DRAM_BASE: u64 = DRAM_BASE;
    pub const DRAM_END: u64 = DRAM_END;

    pub fn new() -> Result<Self, ExecutorError> {
        Ok(Self {
            emulator: Emulator::new(),
        })
    }

    fn validate_memory_access(&self, address: u64, width: usize) -> Result<(), ExecutorError> {
        if width == 0 {
            return Ok(());
        }
        let width_u64 = u64::try_from(width).expect("usize always fits in u64 on supported hosts");
        let in_dram = address >= DRAM_BASE
            && address
                .checked_add(width_u64)
                .is_some_and(|end| end <= DRAM_END);
        if in_dram {
            Ok(())
        } else {
            Err(ExecutorError::MemoryOutOfRange { address, width })
        }
    }

    fn validate_instruction_address(&self, address: u64) -> Result<(), ExecutorError> {
        self.validate_memory_access(address, 4)?;
        if address.is_multiple_of(4) {
            Ok(())
        } else {
            Err(ExecutorError::InvalidInstructionAddress { address })
        }
    }

    fn memory_access_error(
        address: u64,
        width: usize,
        error: impl std::fmt::Debug,
    ) -> ExecutorError {
        ExecutorError::MemoryAccess {
            address,
            width,
            description: format!("{error:?}"),
        }
    }
}

impl Rv64Executor for RvemuExecutor {
    fn load_code(&mut self, machine_code: &[u8]) -> Result<(), ExecutorError> {
        self.validate_memory_access(DRAM_BASE, machine_code.len())?;
        self.emulator.initialize_dram(machine_code.to_vec());
        Ok(())
    }

    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), ExecutorError> {
        self.validate_memory_access(address, bytes.len())?;
        for (offset, byte) in bytes.iter().copied().enumerate() {
            let byte_address = address + offset as u64;
            self.emulator
                .cpu
                .bus
                .write(byte_address, u64::from(byte), 8)
                .map_err(|error| Self::memory_access_error(byte_address, 1, error))?;
        }
        Ok(())
    }

    fn read_memory(&mut self, address: u64, bytes: &mut [u8]) -> Result<(), ExecutorError> {
        self.validate_memory_access(address, bytes.len())?;
        for (offset, byte) in bytes.iter_mut().enumerate() {
            let byte_address = address + offset as u64;
            *byte = self
                .emulator
                .cpu
                .bus
                .read(byte_address, 8)
                .map_err(|error| Self::memory_access_error(byte_address, 1, error))?
                as u8;
        }
        Ok(())
    }

    fn write_register(&mut self, register: IntegerRegister, value: u64) {
        self.emulator
            .cpu
            .xregs
            .write(u64::from(register.index()), value);
    }

    fn read_register(&mut self, register: IntegerRegister) -> u64 {
        self.emulator.cpu.xregs.read(u64::from(register.index()))
    }

    fn execute(
        &mut self,
        entry_pc: u64,
        exit_pc: u64,
        instruction_limit: u64,
    ) -> Result<ExecutionReport, ExecutorError> {
        self.execute_until(entry_pc, &[exit_pc], instruction_limit)
    }

    fn execute_until(
        &mut self,
        entry_pc: u64,
        exit_pcs: &[u64],
        instruction_limit: u64,
    ) -> Result<ExecutionReport, ExecutorError> {
        self.validate_instruction_address(entry_pc)?;
        if exit_pcs.is_empty() {
            return Err(ExecutorError::Initialization {
                description: "execution requires at least one exit PC".to_owned(),
            });
        }
        for exit_pc in exit_pcs {
            self.validate_instruction_address(*exit_pc)?;
        }
        self.emulator.initialize_pc(entry_pc);

        let mut executed_instructions = 0;
        loop {
            // The exit boundary is tested before the next instruction fetch,
            // so the sentinel is neither executed nor counted.
            if exit_pcs.contains(&self.emulator.cpu.pc) {
                return Ok(ExecutionReport {
                    executed_instructions,
                    exit_pc: self.emulator.cpu.pc,
                });
            }
            if executed_instructions == instruction_limit {
                return Err(ExecutorError::InstructionLimitExceeded {
                    limit: instruction_limit,
                    executed_instructions,
                });
            }
            self.emulator
                .cpu
                .execute()
                .map_err(|error| ExecutorError::ExecutionFault {
                    executed_instructions,
                    description: format!("{error:?}"),
                })?;
            // Count only instructions whose execution completed successfully.
            executed_instructions += 1;
        }
    }
}

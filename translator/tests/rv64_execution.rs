use asm_rs::{assemble, Arch};
use ferrum_eth_translator::Opcode;
use revm::{
    db::InMemoryDB,
    primitives::{
        AccountInfo, Address, Bytecode, ExecutionResult, Output, SpecId, TransactTo, U256,
    },
    Evm,
};
use rvemu::{bus::DRAM_BASE, emulator::Emulator};

const HALT_INSTRUCTION: u32 = 0x0010_0073;
const MAX_INSTRUCTIONS: usize = 256;
const RANDOM_SEED: u64 = 0x1234_5678_9abc_def0;
const RANDOM_CASES: usize = 1_000;
const OPERAND_A: u64 = DRAM_BASE + 0x1000;
const OPERAND_B: u64 = DRAM_BASE + 0x1020;
const RESULT: u64 = DRAM_BASE + 0x1040;

fn assemble_generated(opcode: Opcode) -> Result<Vec<u8>, String> {
    let generated = format!(
        "{}\n.globl __rv64_halt\n__rv64_halt:\n    ebreak\n",
        opcode.assembly()
    );
    let assembly = generated
        .lines()
        .filter(|line| !line.trim_start().starts_with('.'))
        .collect::<Vec<_>>()
        .join("\n");
    let program = assemble(&assembly, Arch::Rv64)
        .map_err(|error| format!("could not assemble generated code for RV64IM: {error}"))?;
    if program.len() < 4 || program.len() % 4 != 0 {
        return Err(format!(
            "assembled program has invalid RV64IM text length: {} bytes",
            program.len()
        ));
    }
    let halt = u32::from_le_bytes(
        program[program.len() - 4..]
            .try_into()
            .expect("checked four-byte halt instruction"),
    );
    if halt != HALT_INSTRUCTION {
        return Err(format!(
            "assembled program does not end with EBREAK (found 0x{halt:08x})"
        ));
    }
    if program.len() as u64 >= OPERAND_A - DRAM_BASE {
        return Err("assembled program overlaps the operand memory region".to_owned());
    }
    Ok(program)
}

struct Rv64Runner {
    emulator: Emulator,
    halt_pc: u64,
}

impl Rv64Runner {
    fn new(opcode: Opcode) -> Result<Self, String> {
        let program = assemble_generated(opcode)?;
        let halt_pc = DRAM_BASE + program.len() as u64 - 4;
        let mut emulator = Emulator::new();
        emulator.initialize_dram(program);
        Ok(Self { emulator, halt_pc })
    }

    fn run(&mut self, a: U256, b: U256) -> Result<U256, String> {
        // rvemu's bus access widths are specified in bits, not bytes.
        for (address, value) in [(OPERAND_A, a), (OPERAND_B, b)] {
            for (index, limb) in to_limbs(value).into_iter().enumerate() {
                self.emulator
                    .cpu
                    .bus
                    .write(address + (index as u64 * 8), limb, 64)
                    .map_err(|error| {
                        format!("could not initialize RV64 operand memory: {error:?}")
                    })?;
            }
        }

        self.emulator.cpu.xregs.write(1, self.halt_pc);
        self.emulator.cpu.xregs.write(10, OPERAND_A);
        self.emulator.cpu.xregs.write(11, OPERAND_B);
        self.emulator.cpu.xregs.write(12, RESULT);
        self.emulator.initialize_pc(DRAM_BASE);

        let mut halted = false;
        for _ in 0..MAX_INSTRUCTIONS {
            if self.emulator.cpu.pc == self.halt_pc {
                halted = true;
                break;
            }
            self.emulator
                .cpu
                .execute()
                .map_err(|error| format!("RV64 execution stopped before EBREAK: {error:?}"))?;
        }
        if !halted {
            return Err(format!(
                "RV64 execution did not return to EBREAK within {MAX_INSTRUCTIONS} instructions"
            ));
        }

        let mut result_bytes = [0; 32];
        for index in 0..4 {
            let limb = self
                .emulator
                .cpu
                .bus
                .read(RESULT + (index as u64 * 8), 64)
                .map_err(|error| format!("could not read RV64 result memory: {error:?}"))?;
            let start = 24 - (index * 8);
            result_bytes[start..start + 8].copy_from_slice(&limb.to_be_bytes());
        }
        Ok(U256::from_be_bytes(result_bytes))
    }
}

fn run_evm(opcode: Opcode, a: U256, b: U256) -> Result<U256, String> {
    let mut bytecode = Vec::with_capacity(2 * 33 + 9);
    // EVM binary operations pop the top item first. SUB therefore needs b
    // below a on the stack to evaluate a - b; ADD's order is immaterial.
    let operands = match opcode {
        Opcode::Add => [a, b],
        Opcode::Sub => [b, a],
    };
    for operand in operands {
        bytecode.push(0x7f); // PUSH32
        bytecode.extend_from_slice(&operand.to_be_bytes::<32>());
    }
    let opcode_byte = match opcode {
        Opcode::Add => 0x01,
        Opcode::Sub => 0x03,
    };
    bytecode.extend_from_slice(&[
        opcode_byte,
        0x60,
        0x00, // PUSH1 0
        0x52, // MSTORE
        0x60,
        0x20, // PUSH1 32
        0x60,
        0x00, // PUSH1 0
        0xf3, // RETURN
    ]);

    let contract = Address::from([0x22; 20]);
    let caller = Address::from([0x11; 20]);
    let mut database = InMemoryDB::default();
    database.insert_account_info(
        contract,
        AccountInfo::from_bytecode(Bytecode::new_raw(bytecode.into())),
    );

    let mut evm = Evm::builder()
        .with_db(database)
        .with_spec_id(SpecId::CANCUN)
        .modify_tx_env(|tx| {
            tx.caller = caller;
            tx.transact_to = TransactTo::Call(contract);
            tx.gas_limit = 1_000_000;
        })
        .build();
    let result = evm
        .transact_preverified()
        .map_err(|error| format!("revm transaction failed: {error:?}"))?
        .result;
    match result {
        ExecutionResult::Success {
            output: Output::Call(output),
            ..
        } if output.len() == 32 => {
            let bytes: [u8; 32] = output
                .as_ref()
                .try_into()
                .map_err(|_| "revm returned an output other than 32 bytes".to_owned())?;
            Ok(U256::from_be_bytes(bytes))
        }
        other => Err(format!("revm did not return a 32-byte result: {other:?}")),
    }
}

fn to_limbs(value: U256) -> [u64; 4] {
    let bytes = value.to_be_bytes::<32>();
    std::array::from_fn(|index| {
        let start = 24 - index * 8;
        u64::from_be_bytes(bytes[start..start + 8].try_into().expect("8-byte limb"))
    })
}

fn from_limbs(limbs: [u64; 4]) -> U256 {
    let mut bytes = [0; 32];
    for (index, limb) in limbs.into_iter().enumerate() {
        let start = 24 - index * 8;
        bytes[start..start + 8].copy_from_slice(&limb.to_be_bytes());
    }
    U256::from_be_bytes(bytes)
}

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn random_u256(state: &mut u64) -> U256 {
    let mut bytes = [0; 32];
    for chunk in bytes.chunks_exact_mut(8) {
        chunk.copy_from_slice(&next_random(state).to_be_bytes());
    }
    U256::from_be_bytes(bytes)
}

fn hex(value: U256) -> String {
    format!("0x{:064x}", value)
}

#[test]
fn evm_add_matches_rv64() {
    let max = U256::MAX;
    let cases = [
        ("zero plus zero", U256::ZERO, U256::ZERO),
        ("five plus seven", U256::from(5), U256::from(7)),
        ("maximum plus zero", max, U256::ZERO),
        ("maximum plus one wraps", max, U256::from(1)),
        (
            "carry across the first limb",
            from_limbs([u64::MAX, 0, 0, 0]),
            U256::from(1),
        ),
        (
            "carry across multiple limbs",
            from_limbs([u64::MAX, u64::MAX, 0, 0]),
            U256::from(1),
        ),
        (
            "carry propagation across all four limbs",
            from_limbs([u64::MAX; 4]),
            from_limbs([1; 4]),
        ),
    ];

    let mut rv64 = Rv64Runner::new(Opcode::Add).expect("initialize RV64 execution harness");
    let mut check = |label: &str, a: U256, b: U256| {
        let expected = run_evm(Opcode::Add, a, b).unwrap_or_else(|error| {
            panic!(
                "{label}; seed=0x{RANDOM_SEED:016x}; a={}, b={}: {error}",
                hex(a),
                hex(b)
            )
        });
        let actual = rv64.run(a, b).unwrap_or_else(|error| {
            panic!(
                "{label}; seed=0x{RANDOM_SEED:016x}; a={}, b={}, expected={}: {error}",
                hex(a),
                hex(b),
                hex(expected)
            )
        });
        assert_eq!(
            expected,
            actual,
            "{label}; seed=0x{RANDOM_SEED:016x}; a={}, b={}, expected={}, actual={}",
            hex(a),
            hex(b),
            hex(expected),
            hex(actual)
        );
        (expected, actual)
    };

    for (label, a, b) in cases {
        let (expected, actual) = check(label, a, b);
        if label == "five plus seven" {
            assert_eq!(expected, U256::from(12), "revm ADD(5, 7)");
            assert_eq!(actual, U256::from(12), "RV64 ADD(5, 7)");
            println!("ADD(5, 7)");
            println!("revm result:  {}", hex(expected));
            println!("RV64 result:  {}", hex(actual));
            println!("Parity: PASS");
        }
    }

    let mut state = RANDOM_SEED;
    for index in 0..RANDOM_CASES {
        let a = random_u256(&mut state);
        let b = random_u256(&mut state);
        check(&format!("random case {index}"), a, b);
    }
}

#[test]
fn evm_sub_pushes_b_before_a() {
    let result = run_evm(Opcode::Sub, U256::from(7), U256::from(5))
        .expect("execute revm SUB stack-order check");
    assert_eq!(result, U256::from(2), "EVM SUB must evaluate 7 - 5");
}

#[test]
fn evm_sub_matches_rv64() {
    let max = U256::MAX;
    let cases = [
        ("zero minus zero", U256::ZERO, U256::ZERO, U256::ZERO),
        (
            "seven minus five",
            U256::from(7),
            U256::from(5),
            U256::from(2),
        ),
        (
            "five minus seven wraps",
            U256::from(5),
            U256::from(7),
            max - U256::from(1),
        ),
        ("maximum minus zero", max, U256::ZERO, max),
        ("maximum minus maximum", max, max, U256::ZERO),
        ("zero minus one wraps", U256::ZERO, U256::from(1), max),
        (
            "borrow across the first limb",
            from_limbs([0, 1, 0, 0]),
            U256::from(1),
            from_limbs([u64::MAX, 0, 0, 0]),
        ),
        (
            "borrow across multiple limbs",
            from_limbs([0, 0, 1, 0]),
            U256::from(1),
            from_limbs([u64::MAX, u64::MAX, 0, 0]),
        ),
        (
            "borrow propagation across all four limbs",
            from_limbs([0, 0, 0, 1]),
            U256::from(1),
            from_limbs([u64::MAX, u64::MAX, u64::MAX, 0]),
        ),
    ];

    let mut rv64 = Rv64Runner::new(Opcode::Sub).expect("initialize RV64 execution harness");
    let mut check = |label: &str, a: U256, b: U256, independently_expected: Option<U256>| {
        let expected = run_evm(Opcode::Sub, a, b).unwrap_or_else(|error| {
            panic!(
                "{label}; seed=0x{RANDOM_SEED:016x}; a={}, b={}: {error}",
                hex(a),
                hex(b)
            )
        });
        if let Some(independently_expected) = independently_expected {
            assert_eq!(
                expected,
                independently_expected,
                "{label}; revm disagrees with independently computed result; a={}, b={}",
                hex(a),
                hex(b)
            );
        }
        let actual = rv64.run(a, b).unwrap_or_else(|error| {
            panic!(
                "{label}; seed=0x{RANDOM_SEED:016x}; a={}, b={}, expected={}: {error}",
                hex(a),
                hex(b),
                hex(expected)
            )
        });
        assert_eq!(
            expected,
            actual,
            "{label}; seed=0x{RANDOM_SEED:016x}; a={}, b={}, expected={}, actual={}",
            hex(a),
            hex(b),
            hex(expected),
            hex(actual)
        );
        (expected, actual)
    };

    for (label, a, b, independently_expected) in cases {
        let (expected, actual) = check(label, a, b, Some(independently_expected));
        if label == "seven minus five" || label == "five minus seven wraps" {
            println!("SUB({}, {})", a, b);
            println!("revm result:  {}", hex(expected));
            println!("RV64 result:  {}", hex(actual));
            println!("Parity: PASS");
        }
    }

    let mut state = RANDOM_SEED;
    for index in 0..RANDOM_CASES {
        let a = random_u256(&mut state);
        let b = random_u256(&mut state);
        check(&format!("random case {index}"), a, b, None);
        check(&format!("random reverse case {index}"), b, a, None);
    }
}

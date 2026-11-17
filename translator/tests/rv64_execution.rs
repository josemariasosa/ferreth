use ferrum_eth_translator::{
    execution::{
        directed_cases, hex, random_u256, run_evm, run_evm_bytecode, Rv64Runner,
        DEFAULT_RANDOM_SEED,
    },
    Opcode,
};
use revm::primitives::U256;

const RANDOM_CASES: usize = 1_000;

fn assert_parity(runner: &mut Rv64Runner, opcode: Opcode, label: &str, a: U256, b: U256) {
    let expected = run_evm(opcode, a, b).unwrap_or_else(|error| {
        panic!(
            "{label}; seed=0x{DEFAULT_RANDOM_SEED:016x}; a={}, b={}: {error}",
            hex(a),
            hex(b)
        )
    });
    let actual = runner.run(a, b).unwrap_or_else(|error| {
        panic!(
            "{label}; seed=0x{DEFAULT_RANDOM_SEED:016x}; a={}, b={}, expected={}: {error}",
            hex(a),
            hex(b),
            hex(expected)
        )
    });
    assert_eq!(
        expected,
        actual.result,
        "{label}; seed=0x{DEFAULT_RANDOM_SEED:016x}; a={}, b={}, expected={}, actual={}",
        hex(a),
        hex(b),
        hex(expected),
        hex(actual.result)
    );
    assert_eq!(actual.metrics.total, actual.metrics.category_sum());
}

#[test]
fn evm_add_matches_rv64() {
    let mut rv64 = Rv64Runner::new(Opcode::Add).expect("initialize RV64 execution harness");
    for case in directed_cases() {
        assert_parity(&mut rv64, Opcode::Add, case.label, case.a, case.b);
    }

    let expected = run_evm(Opcode::Add, U256::from(5), U256::from(7)).expect("execute revm ADD");
    let actual = rv64
        .run(U256::from(5), U256::from(7))
        .expect("execute RV64 ADD");
    assert_eq!(expected, U256::from(12));
    assert_eq!(actual.result, U256::from(12));
    println!("ADD(5, 7)");
    println!("revm result:  {}", hex(expected));
    println!("RV64 result:  {}", hex(actual.result));
    println!("Parity: PASS");

    let mut state = DEFAULT_RANDOM_SEED;
    for index in 0..RANDOM_CASES {
        assert_parity(
            &mut rv64,
            Opcode::Add,
            &format!("random case {index}"),
            random_u256(&mut state),
            random_u256(&mut state),
        );
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
    let mut rv64 = Rv64Runner::new(Opcode::Sub).expect("initialize RV64 execution harness");
    for case in directed_cases() {
        assert_parity(&mut rv64, Opcode::Sub, case.label, case.a, case.b);
    }

    let mut state = DEFAULT_RANDOM_SEED;
    for index in 0..RANDOM_CASES {
        let a = random_u256(&mut state);
        let b = random_u256(&mut state);
        assert_parity(
            &mut rv64,
            Opcode::Sub,
            &format!("random case {index}"),
            a,
            b,
        );
        assert_parity(
            &mut rv64,
            Opcode::Sub,
            &format!("random reverse case {index}"),
            b,
            a,
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum TestInstruction {
    Push(U256),
    Add,
    Sub,
}

use TestInstruction::{Add, Push, Sub};

fn run_evm_program(program: &[TestInstruction]) -> Result<U256, String> {
    let mut bytecode = Vec::new();
    for instruction in program {
        match instruction {
            Push(value) => {
                bytecode.push(0x7f);
                bytecode.extend_from_slice(&value.to_be_bytes::<32>());
            }
            Add => bytecode.push(Opcode::Add.evm_byte()),
            Sub => bytecode.push(Opcode::Sub.evm_byte()),
        }
    }
    // Return the top stack value, leaving any lower values untouched.
    bytecode.extend_from_slice(&[0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    run_evm_bytecode(bytecode)
}

struct ProgramRunner {
    add: Rv64Runner,
    sub: Rv64Runner,
}

impl ProgramRunner {
    fn new() -> Result<Self, String> {
        Ok(Self {
            add: Rv64Runner::new(Opcode::Add)?,
            sub: Rv64Runner::new(Opcode::Sub)?,
        })
    }

    fn run(&mut self, program: &[TestInstruction]) -> Result<U256, String> {
        let mut stack = Vec::new();
        for (index, instruction) in program.iter().enumerate() {
            match instruction {
                Push(value) => stack.push(*value),
                Add | Sub => {
                    let a = stack
                        .pop()
                        .ok_or_else(|| format!("stack underflow at instruction {index}"))?;
                    let b = stack
                        .pop()
                        .ok_or_else(|| format!("stack underflow at instruction {index}"))?;
                    let runner = match instruction {
                        Add => &mut self.add,
                        Sub => &mut self.sub,
                        Push(_) => unreachable!(),
                    };
                    stack.push(runner.run(a, b)?.result);
                }
            }
        }
        stack.pop().ok_or_else(|| "empty result stack".to_owned())
    }
}

fn assert_program_parity(
    runner: &mut ProgramRunner,
    program: &[TestInstruction],
    label: &str,
    expected: Option<U256>,
) {
    let evm = run_evm_program(program);
    let rv64 = runner.run(program);
    let context = format!(
        "{label}; seed=0x{DEFAULT_RANDOM_SEED:016x}; program={program:#?}; revm={evm:?}; RV64={rv64:?}"
    );
    let evm = evm.unwrap_or_else(|error| panic!("{context}; revm failed: {error}"));
    let rv64 = rv64.unwrap_or_else(|error| panic!("{context}; RV64 failed: {error}"));
    assert_eq!(evm, rv64, "{context}");
    if let Some(expected) = expected {
        assert_eq!(
            evm,
            expected,
            "{context}; directed expected={}",
            hex(expected)
        );
        assert_eq!(
            rv64,
            expected,
            "{context}; directed expected={}",
            hex(expected)
        );
    }
}

#[test]
fn mixed_arithmetic_directed_programs() {
    let n = |value: u64| U256::from(value);
    let three_limb_max = U256::from_limbs([u64::MAX, u64::MAX, u64::MAX, 0]);
    let cases = [
        (
            "ADD then SUB",
            vec![Push(n(3)), Push(n(7)), Add, Push(n(2)), Sub],
            U256::from_limbs([u64::MAX - 7, u64::MAX, u64::MAX, u64::MAX]),
        ),
        (
            "SUB then ADD",
            vec![Push(n(5)), Push(n(20)), Sub, Push(n(7)), Add],
            n(22),
        ),
        (
            "multiple dependent operations",
            vec![
                Push(n(10)),
                Push(n(25)),
                Add,
                Push(n(5)),
                Sub,
                Push(n(2)),
                Add,
            ],
            U256::from_limbs([u64::MAX - 27, u64::MAX, u64::MAX, u64::MAX]),
        ),
        (
            "overflow then underflow",
            vec![
                Push(n(1)),
                Push(U256::MAX),
                Push(n(1)),
                Add,
                Sub,
                Push(U256::ZERO),
                Add,
            ],
            U256::MAX,
        ),
        (
            "multi-limb carry then borrow",
            vec![Push(n(1)), Push(three_limb_max), Push(n(1)), Add, Sub],
            three_limb_max,
        ),
        (
            "top operand first",
            vec![Push(n(5)), Push(n(7)), Sub, Push(n(3)), Add],
            n(5),
        ),
        (
            "multiple stack values and top result",
            vec![Push(n(999)), Push(n(10)), Push(n(7)), Push(n(50)), Sub, Add],
            n(53),
        ),
    ];
    let mut runner = ProgramRunner::new().expect("initialize RV64 program runners");
    for (label, program, expected) in cases {
        assert_program_parity(&mut runner, &program, label, Some(expected));
    }
}

fn next_program_random(state: &mut u64) -> u64 {
    random_u256(state).as_limbs()[0]
}

#[test]
fn mixed_arithmetic_random_programs() {
    const PROGRAMS: usize = 500;
    let mut runner = ProgramRunner::new().expect("initialize RV64 program runners");
    let mut state = DEFAULT_RANDOM_SEED;
    for index in 0..PROGRAMS {
        let operations = 3 + (next_program_random(&mut state) % 18) as usize;
        let mut program = vec![Push(random_u256(&mut state)), Push(random_u256(&mut state))];
        let mut depth = 2;
        for operation_index in 0..operations {
            if depth < 2 || next_program_random(&mut state) % 4 == 0 {
                program.push(Push(random_u256(&mut state)));
                depth += 1;
            }
            let instruction = match operation_index {
                0 if index % 2 == 0 => Add,
                0 => Sub,
                1 if index % 2 == 0 => Sub,
                1 => Add,
                _ if next_program_random(&mut state) % 2 == 0 => Add,
                _ => Sub,
            };
            program.push(instruction);
            depth -= 1;
        }
        assert_program_parity(
            &mut runner,
            &program,
            &format!("random program {index}"),
            None,
        );
    }
}

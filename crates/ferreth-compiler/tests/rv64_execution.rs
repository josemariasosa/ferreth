use ferreth_compiler::{
    decode,
    execution::{
        directed_cases, hex, random_u256, run_evm, run_evm_bytecode, ProgramRunner, Rv64Runner,
        DEFAULT_RANDOM_SEED,
    },
    Instruction, Opcode,
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
fn revm_reference_uses_cancun_and_reports_execution_errors() {
    // Cancun transient storage is available and returns the stored value.
    let cancun = vec![
        0x60, 0x2a, 0x60, 0x00, 0x5d, 0x60, 0x00, 0x5c, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00,
        0xf3,
    ];
    assert_eq!(run_evm_bytecode(cancun), Ok(U256::from(42)));

    for (bytecode, expected) in [
        (vec![0x01], "StackUnderflow"),
        (vec![0x60, 0x00, 0x60, 0x00, 0xfd], "Revert"),
        (vec![0x00], "Success"),
        (vec![0x60, 0x01, 0x1e], "NotActivated"),
        (vec![0xef, 0x01], "OpcodeNotFound"),
    ] {
        let error = run_evm_bytecode(bytecode).expect_err("reference must reject this result");
        assert!(
            error.contains(expected),
            "expected {expected} in reference error: {error}"
        );
    }
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

#[test]
fn arithmetic_runners_preserve_historical_instruction_metrics() {
    let expected = (34, 8, 4, 9, 8, 4, 1, 0);
    for opcode in [Opcode::Add, Opcode::Sub] {
        let execution = Rv64Runner::new(opcode)
            .and_then(|mut runner| runner.run(U256::from(5), U256::from(7)))
            .expect("execute RV64 arithmetic routine");
        let metrics = execution.metrics;
        assert_eq!(
            (
                metrics.total,
                metrics.loads,
                metrics.stores,
                metrics.arithmetic,
                metrics.comparisons,
                metrics.logical,
                metrics.control_flow,
                metrics.other,
            ),
            expected,
            "{} instruction metrics changed",
            opcode.name()
        );
    }
}

use Instruction::{Add, Push, Sub};

fn run_evm_program(program: &[Instruction]) -> Result<U256, String> {
    let mut bytecode = Vec::new();
    for instruction in program {
        match instruction {
            Instruction::Stop => bytecode.push(0x00),
            Push(value) => {
                bytecode.push(0x7f);
                bytecode.extend_from_slice(&value.to_be_bytes::<32>());
            }
            Add => bytecode.push(Opcode::Add.evm_byte()),
            Sub => bytecode.push(Opcode::Sub.evm_byte()),
        }
    }
    run_evm_arithmetic_bytecode(&bytecode)
}

fn run_evm_arithmetic_bytecode(bytecode: &[u8]) -> Result<U256, String> {
    let mut wrapped = bytecode.to_vec();
    // Return the top stack value, leaving any lower values untouched.
    wrapped.extend_from_slice(&[0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    run_evm_bytecode(wrapped)
}

fn assert_program_parity(
    runner: &mut ProgramRunner,
    program: &[Instruction],
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
fn decoded_sample_matches_revm() {
    let bytecode = [
        0x60, 0x03, 0x60, 0x07, 0x01, 0x60, 0x14, 0x60, 0x05, 0x03, 0x01,
    ];
    let program = decode(&bytecode).expect("decode sample bytecode");
    assert_eq!(
        program,
        vec![
            Push(U256::from(3)),
            Push(U256::from(7)),
            Add,
            Push(U256::from(20)),
            Push(U256::from(5)),
            Sub,
            Add
        ]
    );
    let evm = run_evm_arithmetic_bytecode(&bytecode).expect("execute original bytecode in revm");
    let rv64 = ProgramRunner::new()
        .and_then(|mut runner| runner.run(&program))
        .expect("execute decoded program in RV64");
    let expected = U256::from_limbs([u64::MAX - 4, u64::MAX, u64::MAX, u64::MAX]);
    assert_eq!(evm, expected);
    assert_eq!(rv64, expected);
    println!("decoded sample revm result: {}", hex(evm));
    println!("decoded sample RV64 result: {}", hex(rv64));
}

#[test]
fn decoded_full_width_program_matches_revm_across_limbs() {
    let operands = [
        U256::from_limbs([u64::MAX, u64::MAX, u64::MAX, 0x1234]),
        U256::from_limbs([1, 0, 0, 1]),
        U256::from_limbs([1, 0, 0, 1]),
        U256::from_limbs([0, 0, 0, 0x1236]),
    ];
    let mut bytecode = Vec::new();
    for (index, value) in operands.into_iter().enumerate() {
        bytecode.push(0x7f);
        bytecode.extend_from_slice(&value.to_be_bytes::<32>());
        if index == 1 {
            bytecode.push(0x01);
        }
    }
    bytecode.extend_from_slice(&[0x03, 0x01]);
    let program = decode(&bytecode).expect("decode full-width bytecode");
    let evm = run_evm_arithmetic_bytecode(&bytecode).expect("execute full-width bytecode in revm");
    let rv64 = ProgramRunner::new()
        .and_then(|mut runner| runner.run(&program))
        .expect("execute full-width decoded program in RV64");
    let expected = U256::from_limbs([u64::MAX, u64::MAX, u64::MAX, 0x246a]);
    assert_eq!(evm, expected);
    assert_eq!(rv64, expected);
    println!("full-width revm result: {}", hex(evm));
    println!("full-width RV64 result: {}", hex(rv64));
}

#[test]
fn decoded_program_rejects_stack_underflow() {
    let mut runner = ProgramRunner::new().expect("initialize RV64 program runners");
    for bytecode in [&[0x01][..], &[0x60, 0x01, 0x03][..]] {
        let program = decode(bytecode).expect("decode underflow program");
        let expected_index = program.len() - 1;
        assert_eq!(
            runner.run(&program),
            Err(format!("stack underflow at instruction {expected_index}"))
        );
    }
    assert_eq!(runner.run(&[]), Err("empty result stack".to_owned()));
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

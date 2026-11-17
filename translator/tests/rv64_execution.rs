use ferrum_eth_translator::{
    execution::{directed_cases, hex, random_u256, run_evm, Rv64Runner, DEFAULT_RANDOM_SEED},
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

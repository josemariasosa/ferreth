use alloy_primitives::U256;
use ferreth_compiler::{
    decode,
    execution::{ProgramRunner, Rv64Runner},
    Opcode,
};

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

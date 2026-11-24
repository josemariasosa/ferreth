use ferreth_compiler::{
    compiler::PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
    orchestrator::execute_guest_evm_bytecode,
    rv64::{guest::GuestExecutionStatus, memory::EVM_STACK_CAPACITY},
};
use revm::{
    Context, InspectEvm, Inspector, MainBuilder, MainContext,
    bytecode::Bytecode,
    context::TxEnv,
    context_interface::result::{ExecutionResult, HaltReason},
    database::InMemoryDB,
    interpreter::Interpreter,
    primitives::{Address, U256, hardfork::SpecId},
    state::AccountInfo,
};

const CONTRACT: Address = Address::new([0x22; 20]);
const CALLER: Address = Address::new([0x11; 20]);

#[derive(Debug, Default)]
struct StackInspector {
    stack: Option<Vec<U256>>,
}

impl<CTX> Inspector<CTX> for StackInspector {
    // The test bytecode contains no call opcodes, so each interpreter callback
    // belongs to the invoked contract frame. Seed its initial stack, then
    // retain the post-instruction stack; `step_end` includes an explicit STOP.
    fn initialize_interp(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        self.stack = Some(interpreter.stack.data().clone());
    }

    fn step_end(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        self.stack = Some(interpreter.stack.data().clone());
    }
}

const EXPECTED_NORMAL: (GuestExecutionStatus, u64) = (GuestExecutionStatus::Normal, 0);
const EXPECTED_STACK_UNDERFLOW: (GuestExecutionStatus, u64) = (
    GuestExecutionStatus::GuestException,
    PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
);

#[derive(Debug)]
struct RevmExecution {
    result: ExecutionResult,
    stack: Option<Vec<U256>>,
}

fn execute_revm(bytecode: &[u8]) -> Result<RevmExecution, String> {
    let mut database = InMemoryDB::default();
    database.insert_account_info(
        CONTRACT,
        AccountInfo::default().with_code(Bytecode::new_legacy(bytecode.to_vec().into())),
    );

    let mut evm = Context::mainnet()
        .with_db(database)
        .modify_cfg_chained(|cfg| cfg.spec = SpecId::CANCUN)
        .build_mainnet_with_inspector(StackInspector::default());
    let tx = TxEnv::builder()
        .caller(CALLER)
        .call(CONTRACT)
        .gas_limit(1_000_000)
        .gas_price(0)
        .chain_id(None)
        .build()
        .expect("fixed REVM transaction configuration is valid");

    let result = evm
        .inspect_tx(tx)
        .map_err(|error| format!("{error:?}"))?
        .result;
    let stack = evm.into_inspector().stack;
    Ok(RevmExecution { result, stack })
}

fn format_stack(stack: &[U256]) -> String {
    let values = stack
        .iter()
        .map(|value| format!("0x{value:064x}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn first_differing_index(left: &[U256], right: &[U256]) -> Option<usize> {
    (0..left.len().max(right.len())).find(|&index| left.get(index) != right.get(index))
}

fn bytecode_hex(bytecode: &[u8]) -> String {
    bytecode
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn assert_rc_matches_revm(
    case_name: &str,
    bytecode: &[u8],
    expected_guest: (GuestExecutionStatus, u64),
) -> RevmExecution {
    let revm = execute_revm(bytecode).unwrap_or_else(|error| {
        panic!(
            "{case_name}; bytecode=0x{}; REVM terminal=<host error>; \
             Ferreth terminal=<not executed>; REVM stack=<not available>; \
             Ferreth stack=<not available>; first differing stack index=<not available>; \
             REVM error={error}",
            bytecode_hex(bytecode)
        )
    });
    let revm_stack = revm.stack.as_deref();
    let guest = execute_guest_evm_bytecode(bytecode, u64::MAX).unwrap_or_else(|error| {
        panic!(
            "{case_name}; bytecode=0x{}; REVM result={:?}; \
             Ferreth terminal=<compile/execution error>; REVM stack={}; \
             Ferreth stack=<not available>; first differing stack index=<not available>; \
             compiler/executor error={error}",
            bytecode_hex(bytecode),
            revm.result,
            revm_stack
                .map(format_stack)
                .unwrap_or_else(|| "<not observed>".to_owned()),
        )
    });
    let guest_result = guest.guest_result();
    let guest_terminal = (guest_result.status(), guest_result.exception_code());
    let guest_stack = guest_result.stack();

    let revm_termination_matches = matches!(
        (expected_guest.0, &revm.result),
        (
            GuestExecutionStatus::Normal,
            ExecutionResult::Success { .. }
        ) | (
            GuestExecutionStatus::GuestException,
            ExecutionResult::Halt { .. }
        )
    );
    let both_succeeded =
        matches!(revm.result, ExecutionResult::Success { .. }) && guest_terminal == EXPECTED_NORMAL;
    let differing_index = if both_succeeded {
        // REVM skips interpreter creation for empty deployed bytecode.
        let expected_stack = revm_stack.or_else(|| bytecode.is_empty().then_some(&[]));
        expected_stack.and_then(|stack| first_differing_index(stack, guest_stack))
    } else {
        None
    };

    let missing_stack = both_succeeded && revm_stack.is_none() && !bytecode.is_empty();
    if !revm_termination_matches
        || guest_terminal != expected_guest
        || differing_index.is_some()
        || missing_stack
    {
        let revm_stack = revm_stack
            .map(format_stack)
            .unwrap_or_else(|| "<not observed>".to_owned());
        let first_difference = differing_index
            .map(|index| index.to_string())
            .unwrap_or_else(|| "<none or not comparable>".to_owned());
        panic!(
            "{case_name}: RC-to-REVM mismatch; bytecode=0x{}; \
             expected Ferreth (status, exception code)={expected_guest:?}; \
             REVM result={:?}; Ferreth (status, exception code)={guest_terminal:?}; \
             REVM stack={revm_stack}; Ferreth stack={}; \
             first differing stack index={first_difference}",
            bytecode_hex(bytecode),
            revm.result,
            format_stack(guest_stack),
        );
    }
    revm
}

#[test]
fn directed_rc_programs_match_revm_cancun() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty bytecode", vec![]),
        ("STOP only", vec![0x00]),
        ("PUSH1 followed by STOP", vec![0x60, 0x05, 0x00]),
        ("PUSH1 without explicit STOP", vec![0x60, 0x05]),
        ("truncated PUSH1", vec![0x60]),
        ("multiple PUSH1 instructions", vec![0x60, 0x05, 0x60, 0x03]),
        (
            "STOP followed by unreachable bytecode",
            vec![0x00, 0x60, 0x09, 0x01],
        ),
        (
            "PUSH1 plus PUSH1 plus ADD",
            vec![0x60, 0x05, 0x60, 0x03, 0x01],
        ),
        (
            "PUSH1 plus PUSH1 plus SUB",
            vec![0x60, 0x07, 0x60, 0x05, 0x03],
        ),
        (
            "SUB top-operand ordering",
            vec![0x60, 0x05, 0x60, 0x07, 0x03],
        ),
        (
            "repeated ADD operations",
            vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x60, 0x03, 0x01],
        ),
        (
            "repeated SUB operations",
            vec![0x60, 0x07, 0x60, 0x05, 0x03, 0x60, 0x01, 0x03],
        ),
        (
            "mixed ADD and SUB operations",
            vec![0x60, 0x05, 0x60, 0x03, 0x01, 0x60, 0x02, 0x03],
        ),
        (
            "multiple values remain on the stack",
            vec![0x60, 0x02, 0x60, 0x03, 0x60, 0x04, 0x01],
        ),
        (
            "multi-limb ADD after modular underflow",
            vec![0x60, 0x01, 0x60, 0x00, 0x03, 0x60, 0x00, 0x01],
        ),
        ("U256 modular underflow", vec![0x60, 0x01, 0x60, 0x00, 0x03]),
        (
            "POP retains the lower item",
            vec![0x60, 0x05, 0x60, 0x03, 0x50],
        ),
        ("POP leaves an empty stack", vec![0x60, 0x50, 0x50]),
        ("DUP1 duplicates one item", vec![0x60, 0x05, 0x80]),
        (
            "DUP1 then ADD doubles the top item",
            vec![0x60, 0x05, 0x80, 0x01],
        ),
        (
            "DUP1 preserves lower stack items",
            vec![0x60, 0x05, 0x60, 0x03, 0x80],
        ),
        (
            "DUP1 operand ordering with SUB",
            vec![0x60, 0x05, 0x60, 0x03, 0x80, 0x60, 0x01, 0x03],
        ),
        (
            "DUP1 copies a multi-limb word",
            vec![0x60, 0x01, 0x60, 0x00, 0x03, 0x80],
        ),
        (
            "DUP1 then POP retains the original",
            vec![0x60, 0x05, 0x80, 0x50],
        ),
        (
            "DUP1 then SUB produces zero",
            vec![0x60, 0x05, 0x80, 0x03, 0x00],
        ),
        (
            "DUP1 after STOP is unreachable",
            vec![0x60, 0x05, 0x00, 0x80],
        ),
        ("STOP makes POP unreachable", vec![0x60, 0x05, 0x00, 0x50]),
    ];
    for (name, bytecode) in cases {
        assert_rc_matches_revm(name, &bytecode, EXPECTED_NORMAL);
    }

    let underflow_cases = [
        ("POP with empty stack", vec![0x50]),
        (
            "POP underflows after one successful POP",
            vec![0x60, 0x05, 0x50, 0x50],
        ),
        (
            "POP then ADD underflows",
            vec![0x60, 0x05, 0x60, 0x03, 0x50, 0x01],
        ),
        ("ADD with empty stack", vec![0x01]),
        ("ADD with one operand", vec![0x60, 0x01, 0x01]),
        ("SUB with empty stack", vec![0x03]),
        ("SUB with one operand", vec![0x60, 0x01, 0x03]),
    ];

    for (name, bytecode) in underflow_cases {
        assert_rc_matches_revm(name, &bytecode, EXPECTED_STACK_UNDERFLOW);
    }
}

#[test]
fn revm_43_dup1_empty_stack_reports_stack_overflow_not_underflow() {
    let bytecode = [0x80];
    let revm = execute_revm(&bytecode).expect("execute DUP1 underflow in REVM 43 harness");
    let guest =
        execute_guest_evm_bytecode(&bytecode, u64::MAX).expect("execute guest DUP1 underflow");

    // REVM 43 currently classifies this as StackOverflow; Ferreth follows the
    // EVM stack precondition and publishes StackUnderflow.
    assert!(
        matches!(
            revm.result,
            ExecutionResult::Halt {
                reason: HaltReason::StackOverflow,
                ..
            }
        ),
        "unexpected REVM result: {:?}",
        revm.result
    );
    assert_eq!(
        (
            guest.guest_result().status(),
            guest.guest_result().exception_code()
        ),
        EXPECTED_STACK_UNDERFLOW
    );
}

const RANDOM_SEED: u64 = 0xFE77_E7A0_0000_0001;
const VALID_CASES: usize = 1_000;
const INVALID_CASES: usize = 100;
// At most 12 additional operations, plus reductions and an unreachable tail.
// The 64-byte cap is well below the 1024-PUSH1 assembler resource limit and
// the 1 MiB guest code reservation. The orchestrator budgets at most one
// execution of each emitted RV64 instruction, including the exit publisher.
const MAX_BYTECODE_BYTES: usize = 64;

#[derive(Debug, Clone, Copy)]
enum GeneratedInstruction {
    Push1(u8),
    Add,
    Sub,
    Pop,
    Dup1,
    Stop,
}

impl GeneratedInstruction {
    fn opcode(self) -> u8 {
        match self {
            Self::Push1(_) => 0x60,
            Self::Add => 0x01,
            Self::Sub => 0x03,
            Self::Pop => 0x50,
            Self::Dup1 => 0x80,
            Self::Stop => 0x00,
        }
    }

    fn encode(self, bytecode: &mut Vec<u8>) {
        bytecode.push(self.opcode());
        if let Self::Push1(value) = self {
            bytecode.push(value);
        }
    }
}

struct Generator(u64);

impl Generator {
    // xorshift64 (13, 7, 17), matching the project's benchmark generator.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn push(&mut self, program: &mut Vec<GeneratedInstruction>) {
        let value = self.next();
        program.push(GeneratedInstruction::Push1(if value.is_multiple_of(4) {
            0
        } else {
            value as u8
        }));
    }

    fn binary(&mut self, program: &mut Vec<GeneratedInstruction>) {
        program.push(if self.next().is_multiple_of(2) {
            GeneratedInstruction::Add
        } else {
            GeneratedInstruction::Sub
        });
    }

    fn encode(program: &[GeneratedInstruction]) -> Vec<u8> {
        let mut bytecode = Vec::new();
        for instruction in program {
            instruction.encode(&mut bytecode);
        }
        bytecode
    }

    fn valid_prefix(&mut self) -> (Vec<GeneratedInstruction>, usize) {
        // Each template deliberately starts a required mixed-opcode shape;
        // subsequent choices remain seed-driven and depth-safe.
        let mut program = match self.next() % 5 {
            0 => vec![
                GeneratedInstruction::Push1(5),
                GeneratedInstruction::Dup1,
                GeneratedInstruction::Add,
            ],
            1 => vec![
                GeneratedInstruction::Push1(5),
                GeneratedInstruction::Dup1,
                GeneratedInstruction::Sub,
            ],
            2 => vec![
                GeneratedInstruction::Push1(5),
                GeneratedInstruction::Dup1,
                GeneratedInstruction::Dup1,
            ],
            3 => vec![
                GeneratedInstruction::Push1(5),
                GeneratedInstruction::Push1(3),
                GeneratedInstruction::Add,
                GeneratedInstruction::Pop,
            ],
            _ => vec![
                GeneratedInstruction::Push1(5),
                GeneratedInstruction::Push1(3),
                GeneratedInstruction::Add,
                GeneratedInstruction::Dup1,
            ],
        };
        let mut depth = match program.last() {
            Some(GeneratedInstruction::Pop) => 0,
            Some(GeneratedInstruction::Dup1)
                if matches!(program.get(1), Some(GeneratedInstruction::Dup1)) =>
            {
                3
            }
            Some(GeneratedInstruction::Dup1) => 2,
            _ => 1,
        };
        let additional = (self.next() % 11) as usize;
        for _ in 0..additional {
            match self.next() % 5 {
                0 if depth < EVM_STACK_CAPACITY as usize => {
                    self.push(&mut program);
                    depth += 1;
                }
                1 | 2 if depth >= 2 => {
                    self.binary(&mut program);
                    depth -= 1;
                }
                3 if depth >= 1 => {
                    program.push(GeneratedInstruction::Pop);
                    depth -= 1;
                }
                4 if depth >= 1 && depth < EVM_STACK_CAPACITY as usize => {
                    program.push(GeneratedInstruction::Dup1);
                    depth += 1;
                }
                _ => {
                    self.push(&mut program);
                    depth += 1;
                }
            }
        }
        (program, depth)
    }

    fn valid(&mut self) -> Vec<u8> {
        let (mut program, _) = self.valid_prefix();
        if !self.next().is_multiple_of(3) {
            program.push(GeneratedInstruction::Stop);
            if self.next().is_multiple_of(2) {
                self.push(&mut program);
                self.binary(&mut program);
            }
        }
        Self::encode(&program)
    }

    fn invalid(&mut self) -> Vec<u8> {
        let intended = self.next() % 4;
        let mut program = Vec::new();
        if !self.next().is_multiple_of(2) && intended < 2 {
            self.push(&mut program); // reach binary underflow at depth one.
        }
        program.push(match intended {
            0 => GeneratedInstruction::Add,
            1 => GeneratedInstruction::Sub,
            2 => GeneratedInstruction::Pop,
            _ => GeneratedInstruction::Dup1,
        });
        if self.next().is_multiple_of(2) {
            self.push(&mut program); // unreachable proof tail
            program.push(GeneratedInstruction::Stop);
        }
        Self::encode(&program)
    }
}

#[derive(Debug, Default)]
struct ProgramShape {
    push: usize,
    add: usize,
    sub: usize,
    pop: usize,
    dup1: usize,
    stop: usize,
    zero: usize,
    nonzero: usize,
    max_depth: usize,
    final_depth: usize,
    underflow: Option<(u8, usize)>,
    consecutive_arithmetic: bool,
    distinct_pushes_sub: bool,
    arithmetic_after_dup1: bool,
    pop_after_arithmetic: bool,
    unreachable_tail: bool,
}

// Independently inspect reachable transitions without implementing EVM values.
// Counts include the executed underflowing instruction, but not dead bytes.
fn inspect_shape(bytecode: &[u8]) -> ProgramShape {
    assert!(bytecode.len() <= MAX_BYTECODE_BYTES);
    let mut shape = ProgramShape::default();
    let mut pc = 0;
    let mut previous_arithmetic = false;
    let mut previous_dup1 = false;
    let mut previous_push = None;
    let mut push_pair = None;
    while pc < bytecode.len() {
        let opcode = bytecode[pc];
        pc += 1;
        match opcode {
            0x00 => {
                shape.stop += 1;
                shape.unreachable_tail = pc < bytecode.len();
                break;
            }
            0x60 => {
                let immediate = *bytecode.get(pc).expect("PUSH1 must have an immediate");
                pc += 1;
                shape.push += 1;
                if immediate == 0 {
                    shape.zero += 1;
                } else {
                    shape.nonzero += 1;
                }
                shape.final_depth += 1;
                shape.max_depth = shape.max_depth.max(shape.final_depth);
                assert!(shape.max_depth <= EVM_STACK_CAPACITY as usize);
                previous_arithmetic = false;
                previous_dup1 = false;
                push_pair = previous_push.map(|previous| (previous, immediate));
                previous_push = Some(immediate);
            }
            0x01 | 0x03 => {
                if opcode == 0x01 {
                    shape.add += 1;
                } else {
                    shape.sub += 1;
                }
                shape.consecutive_arithmetic |= previous_arithmetic;
                shape.arithmetic_after_dup1 |= previous_dup1;
                shape.distinct_pushes_sub |=
                    opcode == 0x03 && push_pair.is_some_and(|(a, b)| a != b);
                previous_push = None;
                push_pair = None;
                if shape.final_depth < 2 {
                    shape.underflow = Some((opcode, shape.final_depth));
                    break;
                }
                shape.final_depth -= 1;
                previous_arithmetic = true;
                previous_dup1 = false;
            }
            0x50 => {
                shape.pop += 1;
                shape.pop_after_arithmetic |= previous_arithmetic;
                if shape.final_depth == 0 {
                    shape.underflow = Some((opcode, 0));
                    break;
                }
                shape.final_depth -= 1;
                previous_arithmetic = false;
                previous_dup1 = false;
            }
            0x80 => {
                shape.dup1 += 1;
                if shape.final_depth == 0 {
                    shape.underflow = Some((opcode, 0));
                    break;
                }
                shape.final_depth += 1;
                shape.max_depth = shape.max_depth.max(shape.final_depth);
                previous_arithmetic = false;
                previous_dup1 = true;
            }
            _ => panic!("unexpected generated opcode 0x{opcode:02x} at {pc}"),
        }
    }
    shape
}

#[test]
fn seeded_generator_is_deterministic_and_stack_aware() {
    let mut first = Generator(RANDOM_SEED);
    let mut second = Generator(RANDOM_SEED);
    for index in 0..VALID_CASES {
        let bytecode = first.valid();
        assert_eq!(bytecode, second.valid(), "valid case {index}");
        let shape = inspect_shape(&bytecode);
        assert_eq!(shape.underflow, None, "valid case {index}");
    }
    for index in 0..INVALID_CASES {
        let bytecode = first.invalid();
        assert_eq!(bytecode, second.invalid(), "invalid case {index}");
        let shape = inspect_shape(&bytecode);
        assert!(
            matches!(
                shape.underflow,
                Some((0x01 | 0x03, 0 | 1)) | Some((0x50 | 0x80, 0))
            ),
            "invalid case {index}: {shape:?}"
        );
    }
}

#[test]
fn seeded_random_rc_programs_match_revm_cancun() {
    let mut generator = Generator(RANDOM_SEED);
    let mut counts = ProgramShape::default();
    let mut valid_executed = 0;
    let mut invalid_executed = 0;
    let mut max_bytes = 0;
    let mut fallthrough = 0;
    let mut early_stop = 0;
    let mut multiple_values = 0;
    let mut mixed = 0;
    let mut consecutive = 0;
    let mut distinct_pushes_sub = 0;
    let mut multi_limb = 0;
    let mut nonempty_final = 0;
    let mut empty_final = 0;
    let mut arithmetic_after_dup1 = 0;
    let mut pop_after_arithmetic = 0;
    let mut underflows = [0; 4];

    for (valid, total) in [(true, VALID_CASES), (false, INVALID_CASES)] {
        for index in 0..total {
            let bytecode = if valid {
                generator.valid()
            } else {
                generator.invalid()
            };
            let shape = inspect_shape(&bytecode);
            let category = if valid { "valid" } else { "invalid" };
            let case = format!("seed=0x{RANDOM_SEED:016x}; {category} case index={index}");
            if valid {
                assert_eq!(shape.underflow, None, "{case}");
            } else {
                let (opcode, depth) = shape.underflow.expect("invalid case must reach underflow");
                let slot = match opcode {
                    0x01 => 0,
                    0x03 => 1,
                    0x50 => 2,
                    0x80 => 3,
                    _ => unreachable!(),
                };
                assert!(depth <= 1, "{case}");
                underflows[slot] += 1;
            }
            let revm = assert_rc_matches_revm(
                &case,
                &bytecode,
                if valid {
                    EXPECTED_NORMAL
                } else {
                    EXPECTED_STACK_UNDERFLOW
                },
            );
            if valid {
                valid_executed += 1;
                fallthrough += usize::from(shape.stop == 0);
                early_stop += usize::from(shape.unreachable_tail);
                multiple_values += usize::from(shape.final_depth > 1);
                nonempty_final += usize::from(shape.final_depth > 0);
                empty_final += usize::from(shape.final_depth == 0);
                mixed += usize::from(shape.add > 0 && shape.sub > 0);
                consecutive += usize::from(shape.consecutive_arithmetic);
                distinct_pushes_sub += usize::from(shape.distinct_pushes_sub);
                multi_limb += usize::from(
                    revm.stack
                        .as_ref()
                        .expect("normal generated program has a stack")
                        .iter()
                        .any(|value| *value > U256::from(u64::MAX)),
                );
            } else {
                invalid_executed += 1;
            }
            counts.push += shape.push;
            counts.add += shape.add;
            counts.sub += shape.sub;
            counts.pop += shape.pop;
            counts.dup1 += shape.dup1;
            counts.stop += shape.stop;
            counts.zero += shape.zero;
            counts.nonzero += shape.nonzero;
            arithmetic_after_dup1 += usize::from(shape.arithmetic_after_dup1);
            pop_after_arithmetic += usize::from(shape.pop_after_arithmetic);
            counts.max_depth = counts.max_depth.max(shape.max_depth);
            max_bytes = max_bytes.max(bytecode.len());
        }
    }

    assert_eq!(valid_executed, VALID_CASES);
    assert_eq!(invalid_executed, INVALID_CASES);
    assert!(
        counts.add > 0
            && counts.sub > 0
            && counts.pop > 0
            && counts.dup1 > 0
            && counts.push > 0
            && counts.stop > 0
    );
    assert!(counts.zero > 0 && counts.nonzero > 0);
    assert!(
        fallthrough > 0
            && early_stop > 0
            && multiple_values > 0
            && nonempty_final > 0
            && empty_final > 0
    );
    assert!(mixed > 0 && consecutive > 0 && distinct_pushes_sub > 0 && multi_limb > 0);
    assert!(arithmetic_after_dup1 > 0 && pop_after_arithmetic > 0);
    assert!(underflows.iter().all(|count| *count > 0));
    println!(
        "seed=0x{RANDOM_SEED:016x}; valid={valid_executed}; invalid={invalid_executed}; \
         max_bytes={max_bytes}; max_depth={}; reached opcodes: PUSH1={}, ADD={}, SUB={}, POP={}, DUP1={}, STOP={}; \
         zero={}, nonzero={}; fallthrough={fallthrough}, early_stop={early_stop}, \
         multiple_values={multiple_values}, mixed={mixed}, consecutive={consecutive}, \
         distinct_pushes_sub={distinct_pushes_sub}, multi_limb={multi_limb}, nonempty_final={nonempty_final}, \
         empty_final={empty_final}, arithmetic_after_dup1={}, pop_after_arithmetic={}, \
         underflows={underflows:?}",
        counts.max_depth,
        counts.push,
        counts.add,
        counts.sub,
        counts.pop,
        counts.dup1,
        counts.stop,
        counts.zero,
        counts.nonzero,
        arithmetic_after_dup1,
        pop_after_arithmetic,
    );
}

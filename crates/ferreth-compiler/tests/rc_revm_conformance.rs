use ferreth_compiler::{
    compiler::{
        PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE, PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
    },
    orchestrator::execute_guest_evm_bytecode,
    rv64::{guest::GuestExecutionStatus, memory::EVM_STACK_CAPACITY},
};
use revm::{
    bytecode::Bytecode,
    context::TxEnv,
    context_interface::result::{ExecutionResult, HaltReason},
    database::InMemoryDB,
    interpreter::Interpreter,
    primitives::{hardfork::SpecId, Address, U256},
    state::AccountInfo,
    Context, InspectEvm, Inspector, MainBuilder, MainContext,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Terminal {
    Normal,
    StackUnderflow,
    StackOverflow,
    Other(String),
}

impl Terminal {
    fn from_revm(result: &ExecutionResult) -> Self {
        match result {
            ExecutionResult::Success { .. } => Self::Normal,
            ExecutionResult::Halt {
                reason: HaltReason::StackUnderflow,
                ..
            } => Self::StackUnderflow,
            ExecutionResult::Halt {
                reason: HaltReason::StackOverflow,
                ..
            } => Self::StackOverflow,
            other => Self::Other(format!("REVM {other:?}")),
        }
    }

    fn from_guest(status: GuestExecutionStatus, exception_code: u64) -> Self {
        match (status, exception_code) {
            (GuestExecutionStatus::Normal, 0) => Self::Normal,
            (GuestExecutionStatus::GuestException, PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE) => {
                Self::StackUnderflow
            }
            (GuestExecutionStatus::GuestException, PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE) => {
                Self::StackOverflow
            }
            (status, code) => Self::Other(format!("guest {status:?}, exception_code={code}")),
        }
    }
}

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
    expected: Option<Terminal>,
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
    let revm_terminal = Terminal::from_revm(&revm.result);
    let revm_stack = revm.stack.as_deref();
    let guest = execute_guest_evm_bytecode(bytecode).unwrap_or_else(|error| {
        panic!(
            "{case_name}; bytecode=0x{}; REVM terminal={revm_terminal:?}; \
             Ferreth terminal=<compile/execution error>; REVM stack={}; \
             Ferreth stack=<not available>; first differing stack index=<not available>; \
             compiler/executor error={error}",
            bytecode_hex(bytecode),
            revm_stack
                .map(format_stack)
                .unwrap_or_else(|| "<not observed>".to_owned()),
        )
    });
    let guest_result = guest.guest_result();
    let guest_terminal = Terminal::from_guest(guest_result.status(), guest_result.exception_code());
    let guest_stack = guest_result.stack();

    let differing_index = match (&revm_terminal, &guest_terminal) {
        // REVM short-circuits empty deployed bytecode before creating an
        // interpreter, so there is no reference operand stack to read.
        (Terminal::Normal, Terminal::Normal) if bytecode.is_empty() && revm_stack.is_none() => None,
        (Terminal::Normal, Terminal::Normal) => {
            revm_stack.and_then(|stack| first_differing_index(stack, guest_stack))
        }
        _ => None,
    };

    let missing_stack =
        revm_terminal == Terminal::Normal && revm_stack.is_none() && !bytecode.is_empty();
    if revm_terminal != guest_terminal
        || expected
            .as_ref()
            .is_some_and(|expected| *expected != revm_terminal)
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
             expected terminal={expected:?}; REVM terminal={revm_terminal:?}; \
             Ferreth terminal={guest_terminal:?}; \
             REVM result={:?}; Ferreth status={:?}, exception_code={}; \
             REVM stack={revm_stack}; Ferreth stack={}; \
             first differing stack index={first_difference}",
            bytecode_hex(bytecode),
            revm.result,
            guest_result.status(),
            guest_result.exception_code(),
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
        ("POP with empty stack", vec![0x50]),
        (
            "POP underflows after one successful POP",
            vec![0x60, 0x05, 0x50, 0x50],
        ),
        (
            "POP then ADD underflows",
            vec![0x60, 0x05, 0x60, 0x03, 0x50, 0x01],
        ),
        ("STOP makes POP unreachable", vec![0x60, 0x05, 0x00, 0x50]),
        ("ADD with empty stack", vec![0x01]),
        ("ADD with one operand", vec![0x60, 0x01, 0x01]),
        ("SUB with empty stack", vec![0x03]),
        ("SUB with one operand", vec![0x60, 0x01, 0x03]),
    ];

    for (name, bytecode) in cases {
        assert_rc_matches_revm(name, &bytecode, None);
    }
}

const RANDOM_SEED: u64 = 0xFE77_E7A0_0000_0001;
const VALID_CASES: usize = 1_000;
const INVALID_CASES: usize = 100;
// At most 12 additional operations, plus reductions and an unreachable tail.
// The 64-byte cap is well below the 1024-PUSH1 assembler resource limit and
// the 1 MiB guest code reservation. The orchestrator budgets at most one
// execution of each emitted RV64 instruction, including the exit publisher.
const MAX_BYTECODE_BYTES: usize = 64;

struct Generator(u64);

impl Generator {
    // xorshift64 (13, 7, 17), matching the project's benchmark generator.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn push(&mut self, bytecode: &mut Vec<u8>) {
        let value = self.next();
        bytecode.extend([
            0x60,
            if value.is_multiple_of(4) {
                0
            } else {
                value as u8
            },
        ]);
    }

    fn binary(&mut self, bytecode: &mut Vec<u8>) {
        bytecode.push(if self.next().is_multiple_of(2) {
            0x01
        } else {
            0x03
        });
    }

    fn valid_prefix(&mut self) -> (Vec<u8>, usize) {
        let mut bytecode = Vec::new();
        self.push(&mut bytecode);
        self.push(&mut bytecode);
        self.binary(&mut bytecode);
        let mut depth = 1;
        let additional = (self.next() % 13) as usize;
        for _ in 0..additional {
            if depth < 2 || self.next().is_multiple_of(3) {
                self.push(&mut bytecode);
                depth += 1;
            } else {
                self.binary(&mut bytecode);
                depth -= 1;
            }
        }
        (bytecode, depth)
    }

    fn valid(&mut self) -> Vec<u8> {
        let (mut bytecode, _) = self.valid_prefix();
        if !self.next().is_multiple_of(3) {
            bytecode.push(0x00);
            if self.next().is_multiple_of(2) {
                self.push(&mut bytecode);
                self.binary(&mut bytecode);
            }
        }
        bytecode
    }

    fn invalid(&mut self) -> Vec<u8> {
        let mut bytecode = if self.next().is_multiple_of(2) {
            Vec::new()
        } else {
            let (mut bytecode, mut depth) = self.valid_prefix();
            while depth > 1 {
                self.binary(&mut bytecode);
                depth -= 1;
            }
            bytecode
        };
        self.binary(&mut bytecode);
        if self.next().is_multiple_of(2) {
            self.push(&mut bytecode);
            bytecode.push(0x00);
        }
        bytecode
    }
}

#[derive(Debug, Default)]
struct ProgramShape {
    push: usize,
    add: usize,
    sub: usize,
    stop: usize,
    zero: usize,
    nonzero: usize,
    max_depth: usize,
    final_depth: usize,
    underflow: Option<(u8, usize)>,
    consecutive_arithmetic: bool,
    distinct_pushes_sub: bool,
    unreachable_tail: bool,
}

// Independently inspect reachable transitions without implementing EVM values.
// Counts include the executed underflowing instruction, but not dead bytes.
fn inspect_shape(bytecode: &[u8]) -> ProgramShape {
    assert!(bytecode.len() <= MAX_BYTECODE_BYTES);
    let mut shape = ProgramShape::default();
    let mut pc = 0;
    let mut previous_arithmetic = false;
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
        assert!(shape.add + shape.sub > 0, "valid case {index}");
    }
    for index in 0..INVALID_CASES {
        let bytecode = first.invalid();
        assert_eq!(bytecode, second.invalid(), "invalid case {index}");
        let shape = inspect_shape(&bytecode);
        assert!(
            matches!(shape.underflow, Some((0x01 | 0x03, 0 | 1))),
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
    let mut underflows = [[0; 2]; 2];

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
                assert!(shape.add + shape.sub > 0, "{case}");
            } else {
                let (opcode, depth) = shape.underflow.expect("invalid case must reach underflow");
                underflows[usize::from(opcode == 0x03)][depth] += 1;
            }
            let revm = assert_rc_matches_revm(
                &case,
                &bytecode,
                Some(if valid {
                    Terminal::Normal
                } else {
                    Terminal::StackUnderflow
                }),
            );
            if valid {
                valid_executed += 1;
                fallthrough += usize::from(shape.stop == 0);
                early_stop += usize::from(shape.unreachable_tail);
                multiple_values += usize::from(shape.final_depth > 1);
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
            counts.stop += shape.stop;
            counts.zero += shape.zero;
            counts.nonzero += shape.nonzero;
            counts.max_depth = counts.max_depth.max(shape.max_depth);
            max_bytes = max_bytes.max(bytecode.len());
        }
    }

    assert_eq!(valid_executed, VALID_CASES);
    assert_eq!(invalid_executed, INVALID_CASES);
    assert!(counts.add > 0 && counts.sub > 0 && counts.push > 0 && counts.stop > 0);
    assert!(counts.zero > 0 && counts.nonzero > 0);
    assert!(fallthrough > 0 && early_stop > 0 && multiple_values > 0);
    assert!(mixed > 0 && consecutive > 0 && distinct_pushes_sub > 0 && multi_limb > 0);
    assert!(underflows.iter().flatten().all(|count| *count > 0));
    println!(
        "seed=0x{RANDOM_SEED:016x}; valid={valid_executed}; invalid={invalid_executed}; \
         max_bytes={max_bytes}; max_depth={}; reached opcodes: PUSH1={}, ADD={}, SUB={}, STOP={}; \
         zero={}, nonzero={}; fallthrough={fallthrough}, early_stop={early_stop}, \
         multiple_values={multiple_values}, mixed={mixed}, consecutive={consecutive}, \
         distinct_pushes_sub={distinct_pushes_sub}, multi_limb={multi_limb}, \
         underflows={underflows:?}",
        counts.max_depth,
        counts.push,
        counts.add,
        counts.sub,
        counts.stop,
        counts.zero,
        counts.nonzero
    );
}

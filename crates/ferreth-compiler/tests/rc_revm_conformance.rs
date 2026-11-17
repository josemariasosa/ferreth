use ferreth_compiler::{
    compiler::{
        PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE, PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
    },
    orchestrator::execute_guest_evm_bytecode,
    rv64::guest::GuestExecutionStatus,
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

fn execute_revm(bytecode: &[u8]) -> RevmExecution {
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
        .expect("directed bytecode executes without REVM host/database errors")
        .result;
    let stack = evm.into_inspector().stack;
    RevmExecution { result, stack }
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

fn assert_rc_matches_revm(case_name: &str, bytecode: &[u8]) {
    let revm = execute_revm(bytecode);
    let guest = execute_guest_evm_bytecode(bytecode)
        .unwrap_or_else(|error| panic!("{case_name}: RC compilation/execution failed: {error}"));
    let guest_result = guest.guest_result();
    let revm_terminal = Terminal::from_revm(&revm.result);
    let guest_terminal = Terminal::from_guest(guest_result.status(), guest_result.exception_code());
    let revm_stack = revm.stack.as_deref();
    let guest_stack = guest_result.stack();

    let differing_index = match (&revm_terminal, &guest_terminal) {
        // REVM short-circuits empty deployed bytecode before creating an
        // interpreter, so there is no reference operand stack to read.
        (Terminal::Normal, Terminal::Normal) if bytecode.is_empty() && revm_stack.is_none() => None,
        (Terminal::Normal, Terminal::Normal) => {
            let revm_stack = revm_stack.unwrap_or_else(|| {
                panic!(
                    "{case_name}: REVM completed normally without an observed interpreter stack; \
                     bytecode=0x{}; REVM terminal={revm_terminal:?}; \
                     Ferreth terminal={guest_terminal:?}; REVM result={:?}; \
                     Ferreth status={:?}, exception_code={}; REVM stack=<not observed>; \
                     Ferreth stack={}",
                    bytecode_hex(bytecode),
                    revm.result,
                    guest_result.status(),
                    guest_result.exception_code(),
                    format_stack(guest_stack),
                )
            });
            first_differing_index(revm_stack, guest_stack)
        }
        _ => None,
    };

    if revm_terminal != guest_terminal || differing_index.is_some() {
        let revm_stack = revm_stack
            .map(format_stack)
            .unwrap_or_else(|| "<not observed>".to_owned());
        let first_difference = differing_index
            .map(|index| format!("; first differing stack index: {index}"))
            .unwrap_or_default();
        panic!(
            "{case_name}: RC-to-REVM mismatch; bytecode=0x{}; \
             REVM terminal={revm_terminal:?}; Ferreth terminal={guest_terminal:?}; \
             REVM result={:?}; Ferreth status={:?}, exception_code={}; \
             REVM stack={revm_stack}; Ferreth stack={};{first_difference}",
            bytecode_hex(bytecode),
            revm.result,
            guest_result.status(),
            guest_result.exception_code(),
            format_stack(guest_stack),
        );
    }
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
        ("ADD with empty stack", vec![0x01]),
        ("ADD with one operand", vec![0x60, 0x01, 0x01]),
        ("SUB with empty stack", vec![0x03]),
        ("SUB with one operand", vec![0x60, 0x01, 0x03]),
    ];

    for (name, bytecode) in cases {
        assert_rc_matches_revm(name, &bytecode);
    }
}

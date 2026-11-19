pub mod benchmark;

use ferreth_compiler::Opcode;
use revm::{
    Context, ExecuteEvm, MainBuilder, MainContext,
    bytecode::Bytecode,
    context::TxEnv,
    context_interface::result::{ExecutionResult, Output},
    database::InMemoryDB,
    primitives::{Address, U256, hardfork::SpecId},
    state::AccountInfo,
};

pub fn run_evm(opcode: Opcode, a: U256, b: U256) -> Result<U256, String> {
    let mut bytecode = Vec::with_capacity(2 * 33 + 9);
    // EVM binary operations pop the top item first. SUB therefore needs b
    // below a on the stack to evaluate a - b; ADD's order is immaterial.
    let operands = match opcode {
        Opcode::Add => [a, b],
        Opcode::Sub => [b, a],
    };
    for operand in operands {
        bytecode.push(0x7f);
        bytecode.extend_from_slice(&operand.to_be_bytes::<32>());
    }
    bytecode.extend_from_slice(&[
        opcode.evm_byte(),
        0x60,
        0x00,
        0x52, // PUSH1 0; MSTORE
        0x60,
        0x20,
        0x60,
        0x00,
        0xf3, // PUSH1 32; PUSH1 0; RETURN
    ]);

    run_evm_bytecode(bytecode)
}

/// Executes bytecode in the same fixed environment used by the arithmetic
/// differential tests and returns its 32-byte call output.
pub fn run_evm_bytecode(bytecode: Vec<u8>) -> Result<U256, String> {
    let contract = Address::from([0x22; 20]);
    let caller = Address::from([0x11; 20]);
    let mut database = InMemoryDB::default();
    database.insert_account_info(
        contract,
        AccountInfo::default().with_code(Bytecode::new_legacy(bytecode.into())),
    );
    let mut evm = Context::mainnet()
        .with_db(database)
        .modify_cfg_chained(|cfg| cfg.spec = SpecId::CANCUN)
        .build_mainnet();
    let tx = TxEnv::builder()
        .caller(caller)
        .call(contract)
        .gas_limit(1_000_000)
        .gas_price(0)
        .chain_id(None)
        .build()
        .map_err(|error| format!("revm transaction configuration failed: {error:?}"))?;
    let result = evm
        .transact(tx)
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

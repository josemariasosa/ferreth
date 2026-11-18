use std::env;

use revm::bytecode::Bytecode;
use revm::context_interface::host::DummyHost;
use revm::interpreter::{
    InputsImpl, Interpreter, InterpreterAction, SharedMemory, instruction_table,
    instructions::gas_table_spec,
    interpreter::{EthInterpreter, ExtBytecode},
    interpreter_types::Jumps,
};
use revm::primitives::{Bytes, hardfork::SpecId};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut code = None;
    let mut gas = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--code" => {
                code = Some(args.next().ok_or("Missing bytecode")?);
            }
            "--gas" => {
                gas = Some(
                    args.next()
                        .ok_or("Missing gas")?
                        .parse::<u64>()
                        .map_err(|_| "Invalid gas")?,
                );
            }
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }

    let code = code.ok_or("Required argument: --code")?;
    let initial_gas = gas.ok_or("Required argument: --gas")?;
    let code = code.strip_prefix("0x").unwrap_or(&code);

    if !code.is_ascii() || !code.bytes().all(|b| b.is_ascii_hexdigit()) || code.len() % 2 != 0 {
        return Err("Invalid hexadecimal bytecode".into());
    }

    let raw_code = (0..code.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&code[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Invalid hexadecimal bytecode")?;

    let spec = SpecId::CANCUN;
    let bytecode = Bytecode::new_raw(Bytes::from(raw_code));

    let mut interpreter = Interpreter::<EthInterpreter>::new(
        SharedMemory::new(),
        ExtBytecode::new(bytecode),
        InputsImpl::default(),
        false,
        spec,
        initial_gas,
    );

    let instructions = instruction_table::<EthInterpreter, DummyHost>();

    let gas_table = gas_table_spec(spec);
    let mut host = DummyHost::new(spec);

    let action = interpreter.run_plain(&instructions, &gas_table, &mut host);

    match action {
        InterpreterAction::Return(result) => {
            let remaining_gas = result.gas.remaining();
            let success = result.is_ok();

            println!("Bytecode: {code}");
            println!("Initial Msg gas: {initial_gas}");
            println!("Raw remaining gas: {remaining_gas}");
            println!("Raw gas consumed: {}", initial_gas - remaining_gas);
            println!("Stack: {:?}", interpreter.stack.data());
            println!("PC: {}", interpreter.bytecode.pc());
            println!("Success: {success}");
            println!("Result: {:?}", result.result);

            if success {
                println!("Error: None");
            } else {
                println!("Error: {:?}", result.result);
            }
        }
        other => {
            return Err(format!("Unsupported interpreter action: {other:?}"));
        }
    }

    Ok(())
}

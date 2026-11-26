use std::process::ExitCode;

use alloy_primitives::U256;
use ferreth_compiler::{
    compiler::{
        OUT_OF_GAS_EXCEPTION_CODE, PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE,
        PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE,
    },
    decode,
    disassembler::{disassemble, format_disassembly, parse_hex},
    orchestrator::{
        execute_guest_add, execute_guest_dup1, execute_guest_evm_bytecode, execute_guest_pop,
        execute_guest_push1, execute_guest_sub,
    },
    rv64::{guest::GuestExecutionStatus, opcodes::push::generate_guest_push1},
    Opcode,
};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("translate") => translate(&args),
        Some("execute") => execute(&args[1..]),
        Some("decode") => decode_command(&args[1..]),
        _ => usage_failure(),
    }
}

fn translate(args: &[String]) -> ExitCode {
    match args {
        [command, option, operation] if command == "translate" && option == "--op" => {
            let opcode = match operation.as_str() {
                "ADD" => Opcode::Add,
                "SUB" => Opcode::Sub,
                _ => return usage_failure(),
            };
            print!("{}", opcode.assembly());
            ExitCode::SUCCESS
        }
        [command, option, operation, value_option, value]
            if command == "translate"
                && option == "--op"
                && operation == "PUSH1"
                && value_option == "--value" =>
        {
            let Some(immediate) = parse_push1_value(value) else {
                return usage_failure();
            };
            let generated = generate_guest_push1(immediate);
            println!("PUSH1 opcode body (unlinked; overflow trampoline is patchable):");
            print!("{}", generated.assembly());
            println!("Machine-code words:");
            for word in generated.instruction_words() {
                println!("  0x{word:08x}");
            }
            ExitCode::SUCCESS
        }
        _ => usage_failure(),
    }
}

fn execute(args: &[String]) -> ExitCode {
    match args {
        [option, operation] if option == "--op" && operation == "POP" => {
            match execute_guest_pop() {
                Ok(result) => {
                    println!("Opcode: POP");
                    println!("Final guest status: {:?}", result.guest_result().status());
                    println!("Final stack: {:?}", result.guest_result().stack());
                    println!(
                        "Dynamic RV64 instruction count: {}",
                        result.instruction_metrics().total()
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("POP execution failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [option, operation] if option == "--op" && operation == "DUP1" => {
            match execute_guest_dup1() {
                Ok(result) => {
                    println!("Opcode: DUP1");
                    println!("Final guest status: {:?}", result.guest_result().status());
                    println!("Final stack: {:?}", result.guest_result().stack());
                    println!(
                        "Dynamic RV64 instruction count: {}",
                        result.instruction_metrics().total()
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("DUP1 execution failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [option, operation, a_option, a, b_option, b]
            if option == "--op"
                && matches!(operation.as_str(), "ADD" | "SUB")
                && a_option == "--a"
                && b_option == "--b" =>
        {
            let (Some(a), Some(b)) = (parse_u256(a), parse_u256(b)) else {
                return usage_failure();
            };
            let execution = match operation.as_str() {
                "ADD" => execute_guest_add(a, b),
                "SUB" => execute_guest_sub(a, b),
                _ => unreachable!("match guard accepts only ADD or SUB"),
            };
            match execution {
                Ok(result) => {
                    println!("Opcode: {operation}");
                    println!("Initial stack: [{a:?}, {b:?}]");
                    println!("Final guest status: {:?}", result.guest_result().status());
                    println!("Final stack: {:?}", result.guest_result().stack());
                    println!(
                        "Dynamic RV64 instruction count: {}",
                        result.instruction_metrics().total()
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{operation} execution failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [option, bytecode, budget @ ..] if option == "--bytecode" => {
            let gas_limit = match budget {
                [] => u64::MAX,
                [flag, value] if flag == "--gas" => {
                    let Some(gas_limit) = parse_u64(value) else {
                        return usage_failure();
                    };
                    gas_limit
                }
                _ => return usage_failure(),
            };
            let bytecode = match parse_hex(bytecode) {
                Ok(bytecode) => bytecode,
                Err(error) => {
                    eprintln!("invalid bytecode hex: {error}");
                    return usage_failure();
                }
            };
            match (
                decode(&bytecode),
                execute_guest_evm_bytecode(&bytecode, gas_limit),
            ) {
                (Ok(instructions), Ok(result)) => {
                    let guest = result.guest_result();
                    println!("Decoded instructions: {instructions:?}");
                    println!("Final status: {:?}", guest.status());
                    if guest.status() == GuestExecutionStatus::GuestException {
                        let code = guest.exception_code();
                        let name = match code {
                            PROVISIONAL_STACK_OVERFLOW_EXCEPTION_CODE => "StackOverflow",
                            PROVISIONAL_STACK_UNDERFLOW_EXCEPTION_CODE => "StackUnderflow",
                            OUT_OF_GAS_EXCEPTION_CODE => "OutOfGas",
                            _ => unreachable!("GuestResult validates exception codes"),
                        };
                        println!("Guest exception: {name} ({code})");
                    }
                    println!("Initial frame gas: {}", guest.gas_limit());
                    println!("Remaining frame gas: {}", guest.remaining_gas());
                    println!("Gross gas used: {}", guest.gas_used());
                    println!("Final stack: {:?}", guest.stack());
                    println!(
                        "Dynamic RV64 instruction count: {}",
                        result.instruction_metrics().total()
                    );
                    ExitCode::SUCCESS
                }
                (Err(error), _) => {
                    eprintln!("bytecode decoding failed: {error}");
                    ExitCode::FAILURE
                }
                (_, Err(error)) => {
                    eprintln!("bytecode execution failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [option, operation, value_option, value]
            if option == "--op" && operation == "PUSH1" && value_option == "--value" =>
        {
            let Some(immediate) = parse_push1_value(value) else {
                return usage_failure();
            };
            match execute_guest_push1(immediate) {
                Ok(result) => {
                    println!("PUSH1 {immediate} complete linked executable program:");
                    print!("{}", result.assembly());
                    println!("Machine-code words:");
                    for word in result.machine_code().as_chunks::<4>().0 {
                        println!("  0x{:08x}", u32::from_le_bytes(*word));
                    }
                    println!("Final stack: {:?}", result.guest_result().stack());
                    println!(
                        "Instructions: {} (opcode body {}, termination {})",
                        result.instruction_metrics().total(),
                        result.instruction_metrics().opcode_body(),
                        result.instruction_metrics().guest_termination(),
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("PUSH1 execution failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage_failure(),
    }
}

fn decode_command(args: &[String]) -> ExitCode {
    let [option, hex] = args else {
        return usage_failure();
    };
    if option != "--bytecode" {
        return usage_failure();
    }
    match parse_hex(hex) {
        Ok(bytecode) => {
            print!("{}", format_disassembly(&disassemble(&bytecode)));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("invalid bytecode hex: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_u64(value: &str) -> Option<u64> {
    value.strip_prefix("0x").map_or_else(
        || value.parse().ok(),
        |hex| u64::from_str_radix(hex, 16).ok(),
    )
}

fn parse_push1_value(value: &str) -> Option<u8> {
    u8::try_from(parse_u64(value)?).ok()
}

fn parse_u256(value: &str) -> Option<U256> {
    value.parse().ok()
}

fn usage_failure() -> ExitCode {
    eprintln!("Usage:\n  ferreth-compiler translate --op ADD|SUB\n  ferreth-compiler translate --op PUSH1 --value 0..255\n  ferreth-compiler execute --op ADD|SUB --a U256 --b U256\n  ferreth-compiler execute --op POP|DUP1\n  ferreth-compiler execute --op PUSH1 --value 0..255\n  ferreth-compiler execute --bytecode HEX [--gas U64]\n  ferreth-compiler decode --bytecode HEX");
    ExitCode::FAILURE
}

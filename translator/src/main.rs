use ferrum_eth_translator::{
    benchmark::{run_benchmark, BenchOptions},
    execution::DEFAULT_RANDOM_SEED,
    orchestrator::{execute_guest_push1, execute_guest_push1_sequence},
    rv64::push::generate_guest_push1,
    Opcode,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("translate") => translate(&args),
        Some("execute") => execute(&args[1..]),
        Some("bench") => bench(&args[1..]),
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
        [option, values] if option == "--push1-sequence" => {
            let Some(immediates) = parse_push1_sequence(values) else {
                return usage_failure();
            };
            match execute_guest_push1_sequence(&immediates) {
                Ok(result) => {
                    println!("PUSH1 sequence {immediates:?} complete linked executable program:");
                    print!("{}", result.assembly());
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
                    eprintln!("PUSH1 sequence execution failed: {error}");
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
                    for word in result.machine_code().chunks_exact(4) {
                        println!(
                            "  0x{:08x}",
                            u32::from_le_bytes(word.try_into().expect("word"))
                        );
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

fn bench(args: &[String]) -> ExitCode {
    let mut operation = None;
    let mut cases = None;
    let mut seed = DEFAULT_RANDOM_SEED;
    let mut format = None;
    let mut index = 0;
    while index < args.len() {
        let Some(value) = args.get(index + 1) else {
            return usage_failure();
        };
        match args[index].as_str() {
            "--op" if operation.is_none() => operation = Some(value.as_str()),
            "--cases" if cases.is_none() => match value.parse::<usize>() {
                Ok(parsed) => cases = Some(parsed),
                Err(_) => return usage_failure(),
            },
            "--seed" => match parse_u64(value) {
                Some(parsed) => seed = parsed,
                None => return usage_failure(),
            },
            "--format" if format.is_none() => format = Some(value.as_str()),
            _ => return usage_failure(),
        }
        index += 2;
    }
    let (Some(operation), Some(cases)) = (operation, cases) else {
        return usage_failure();
    };
    if format != Some("json") {
        return usage_failure();
    }
    let operations = match operation {
        "ALL" => vec![Opcode::Add, Opcode::Sub],
        "ADD" => vec![Opcode::Add],
        "SUB" => vec![Opcode::Sub],
        _ => return usage_failure(),
    };
    let report = run_benchmark(BenchOptions {
        operations,
        random_cases_per_operation: cases,
        seed,
    });
    let passed = report.passed();
    // JSON is the sole stdout payload for evidence capture. A failed report is
    // still emitted before returning failure so its structured evidence survives.
    println!("{}", report.to_json());
    if passed {
        ExitCode::SUCCESS
    } else {
        eprintln!("benchmark differential parity failure; see JSON failures array");
        ExitCode::FAILURE
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

fn parse_push1_sequence(value: &str) -> Option<Vec<u8>> {
    value.split(',').map(parse_push1_value).collect()
}

fn usage_failure() -> ExitCode {
    eprintln!("Usage:\n  ferrum-eth-translator translate --op ADD|SUB\n  ferrum-eth-translator translate --op PUSH1 --value 0..255\n  ferrum-eth-translator execute --op PUSH1 --value 0..255\n  ferrum-eth-translator execute --push1-sequence 5,3\n  ferrum-eth-translator bench --op ADD|SUB|ALL --cases N [--seed N] --format json");
    ExitCode::FAILURE
}

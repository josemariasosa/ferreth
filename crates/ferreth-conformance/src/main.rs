use std::process::ExitCode;

use ferreth_compiler::Opcode;
use ferreth_compiler::execution::DEFAULT_RANDOM_SEED;

use ferreth_conformance::benchmark::{BenchOptions, run_benchmark, run_guest_benchmark};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bench") => bench(&args[1..]),
        _ => usage_failure(),
    }
}

fn bench(args: &[String]) -> ExitCode {
    if matches!(args, [scope, format] if scope == "guest" && format == "--format=json")
        || matches!(args, [scope, flag, format] if scope == "guest" && flag == "--format" && format == "json")
    {
        return match run_guest_benchmark() {
            Ok(report) => {
                println!("{}", report.to_json());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("guest benchmark failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
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

#[allow(dead_code)]
fn parse_u256(value: &str) -> Option<revm::primitives::U256> {
    value.parse().ok()
}

#[allow(dead_code)]
fn parse_push1_sequence(value: &str) -> Option<Vec<u8>> {
    value.split(',').map(parse_push1_value).collect()
}

#[allow(dead_code)]
fn parse_bytecode(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn usage_failure() -> ExitCode {
    eprintln!(
        "Usage:\n  ferreth-conformance bench --op ADD|SUB|ALL --cases N [--seed N] --format json\n  ferreth-conformance bench guest --format json"
    );
    ExitCode::FAILURE
}

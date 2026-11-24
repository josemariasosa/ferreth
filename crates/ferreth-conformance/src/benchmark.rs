//! Deterministic evidence collection for the RV64 arithmetic baseline.

use std::fmt::Write;

use revm::primitives::U256;

use crate::run_evm;
use ferreth_compiler::{
    Opcode,
    execution::{InstructionMetrics, Rv64Runner, directed_cases, hex, random_u256},
    orchestrator::{
        EXECUTOR_IDENTITY, OrchestratorError, OrchestratorResult, execute_guest_add,
        execute_guest_dup1, execute_guest_evm_bytecode, execute_guest_pop, execute_guest_push1,
        execute_guest_sub,
    },
    rv64::guest::GuestExecutionStatus,
};

/// Deterministic RV64 guest measurements. Unlike [`BenchmarkReport`], this
/// is a separate schema so the historical arithmetic evidence stays byte-for-byte stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestBenchmarkReport {
    opcode_guest: Vec<GuestMeasurement>,
    program_guest: Vec<GuestProgramMeasurement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuestMeasurement {
    name: &'static str,
    metrics: ferreth_compiler::orchestrator::GuestInstructionMetrics,
    status: GuestExecutionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuestProgramMeasurement {
    bytecode: &'static str,
    metrics: ferreth_compiler::orchestrator::GuestInstructionMetrics,
    status: GuestExecutionStatus,
}

impl GuestBenchmarkReport {
    /// Serializes the guest-only evidence schema without changing the
    /// historical `rv64-arithmetic-baseline` schema.
    pub fn to_json(&self) -> String {
        let mut output =
            String::from("{\"schema_version\":1,\"experiment\":\"rv64-guest-instruction-metrics\"");
        output.push_str(",\"executor\":\"");
        output.push_str(EXECUTOR_IDENTITY);
        output.push_str("\",\"measurement\":\"successfully executed RV64 instructions; exit sentinel observed before fetch and excluded\"");
        output.push_str(",\"opcode_guest\":[");
        for (index, measurement) in self.opcode_guest.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            write_guest_measurement(
                &mut output,
                measurement.name,
                measurement.status,
                measurement.metrics,
            );
        }
        output.push_str("],\"program_guest\":[");
        for (index, measurement) in self.program_guest.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            write_guest_measurement(
                &mut output,
                measurement.bytecode,
                measurement.status,
                measurement.metrics,
            );
        }
        output.push_str("]}");
        output
    }
}

fn write_guest_measurement(
    output: &mut String,
    name: &str,
    status: GuestExecutionStatus,
    metrics: ferreth_compiler::orchestrator::GuestInstructionMetrics,
) {
    write!(output, "{{\"input\":\"{name}\",\"status\":\"{}\",\"rv64\":{{\"opcode_body\":{},\"guest_termination\":{},\"total\":{}}}}}", guest_status_name(status), metrics.opcode_body(), metrics.guest_termination(), metrics.total()).expect("writing JSON to String");
}

fn guest_status_name(status: GuestExecutionStatus) -> &'static str {
    match status {
        GuestExecutionStatus::Running => "running",
        GuestExecutionStatus::Normal => "normal",
        GuestExecutionStatus::GuestException => "guest_exception",
    }
}

/// Measures representative guest opcode and whole-program executions. Rust
/// only supplies input bytes and initial operands; each EVM operation executes
/// in the linked RV64 guest.
pub fn run_guest_benchmark() -> Result<GuestBenchmarkReport, OrchestratorError> {
    let opcode_guest = [
        ("PUSH1 0x05", execute_guest_push1(5)),
        (
            "ADD 0x05 0x03",
            execute_guest_add(U256::from(5), U256::from(3)),
        ),
        (
            "SUB 0x05 0x03",
            execute_guest_sub(U256::from(5), U256::from(3)),
        ),
        ("POP [0x00]", execute_guest_pop()),
        ("DUP1 [0x00]", execute_guest_dup1()),
    ]
    .into_iter()
    .map(|(name, result)| guest_measurement(name, result))
    .collect::<Result<Vec<_>, _>>()?;
    let program_guest = [
        "6005",
        "60056003",
        "6005600301",
        "6005600303",
        "600560030100",
        "600550",
        "600580",
        "60058001",
        "6005600350",
        "60058050",
    ]
    .into_iter()
    .map(|bytecode| {
        let bytes = (0..bytecode.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&bytecode[index..index + 2], 16).expect("fixed hex"))
            .collect::<Vec<_>>();
        guest_program_measurement(bytecode, execute_guest_evm_bytecode(&bytes, u64::MAX))
    })
    .collect::<Result<Vec<_>, _>>()?;
    Ok(GuestBenchmarkReport {
        opcode_guest,
        program_guest,
    })
}

fn guest_measurement(
    name: &'static str,
    result: Result<OrchestratorResult, OrchestratorError>,
) -> Result<GuestMeasurement, OrchestratorError> {
    let result = result?;
    Ok(GuestMeasurement {
        name,
        metrics: result.instruction_metrics(),
        status: result.guest_result().status(),
    })
}

fn guest_program_measurement(
    bytecode: &'static str,
    result: Result<OrchestratorResult, OrchestratorError>,
) -> Result<GuestProgramMeasurement, OrchestratorError> {
    let result = result?;
    Ok(GuestProgramMeasurement {
        bytecode,
        metrics: result.instruction_metrics(),
        status: result.guest_result().status(),
    })
}

#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub operations: Vec<Opcode>,
    pub random_cases_per_operation: usize,
    pub seed: u64,
}

#[derive(Debug, Clone)]
pub struct BenchmarkReport {
    options: BenchOptions,
    operations: Vec<OperationReport>,
}

impl BenchmarkReport {
    pub fn passed(&self) -> bool {
        self.operations
            .iter()
            .all(|operation| operation.failed == 0)
    }

    /// Serializes the versioned evidence schema without a serialization
    /// dependency. All dynamic strings are JSON escaped.
    pub fn to_json(&self) -> String {
        let mut output = String::new();
        output.push('{');
        push_json_field_number(&mut output, "schema_version", 1, false);
        push_json_field_string(&mut output, "experiment", "rv64-arithmetic-baseline", true);
        output.push_str(",\"configuration\":{");
        push_json_field_number(&mut output, "seed", self.options.seed, false);
        push_json_field_number(
            &mut output,
            "random_cases_per_operation",
            self.options.random_cases_per_operation as u64,
            true,
        );
        push_json_field_string(&mut output, "random_generator", "xorshift64", true);
        push_json_field_string(&mut output, "emulator", "rvemu", true);
        push_json_field_string(&mut output, "evm_reference", "revm", true);
        push_json_field_string(&mut output, "evm_spec", "CANCUN", true);
        output.push_str(",\"dependencies\":{");
        push_json_field_string(&mut output, "asm_rs", "0.2.0", false);
        push_json_field_string(&mut output, "revm", "43.0.3", true);
        push_json_field_string(&mut output, "rvemu", "0.0.11", true);
        output.push_str("}}");
        output.push_str(",\"operations\":[");
        for (index, operation) in self.operations.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            operation.write_json(&mut output);
        }
        output.push_str("]}");
        output
    }
}

#[derive(Debug, Clone)]
struct OperationReport {
    opcode: Opcode,
    directed: u64,
    random: u64,
    passed: u64,
    failed: u64,
    machine_code_hash: Option<u64>,
    routine_instruction_words: Option<u64>,
    executed_cases: u64,
    metrics: InstructionMetrics,
    min_instructions: Option<u64>,
    max_instructions: Option<u64>,
    failures: Vec<FailureEvidence>,
}

impl OperationReport {
    fn write_json(&self, output: &mut String) {
        output.push('{');
        push_json_field_string(output, "name", self.opcode.name(), false);
        push_json_field_string(
            output,
            "evm_opcode",
            &format!("0x{:02x}", self.opcode.evm_byte()),
            true,
        );
        output.push_str(",\"cases\":{");
        push_json_field_number(output, "directed", self.directed, false);
        push_json_field_number(output, "random", self.random, true);
        push_json_field_number(output, "total", self.directed + self.random, true);
        push_json_field_number(output, "passed", self.passed, true);
        push_json_field_number(output, "failed", self.failed, true);
        output.push('}');
        output.push_str(",\"rv64\":{");
        push_json_field_string(
            output,
            "measurement",
            "decoded 32-bit instructions successfully executed by rvemu; harness EBREAK excluded",
            false,
        );
        push_json_field_number(output, "executed_cases", self.executed_cases, true);
        push_json_optional_hex_u64(output, "machine_code_fnv1a64", self.machine_code_hash, true);
        push_json_optional_number(
            output,
            "routine_instruction_words",
            self.routine_instruction_words,
            true,
        );
        output.push_str(",\"instructions_per_case\":{");
        push_json_optional_number(output, "min", self.min_instructions, false);
        push_json_optional_number(output, "max", self.max_instructions, true);
        output.push_str(",\"mean\":");
        if self.executed_cases == 0 {
            output.push_str("null");
        } else {
            let mean = self.metrics.total as f64 / self.executed_cases as f64;
            write!(output, "{mean:.6}").expect("writing JSON to String");
        }
        output.push('}');
        output.push_str(",\"instruction_categories\":{");
        push_json_field_string(output, "scope", "aggregate across executed cases", false);
        push_json_field_number(output, "total_executed", self.metrics.total, true);
        push_json_field_number(output, "loads", self.metrics.loads, true);
        push_json_field_number(output, "stores", self.metrics.stores, true);
        push_json_field_number(output, "arithmetic", self.metrics.arithmetic, true);
        push_json_field_number(output, "comparisons", self.metrics.comparisons, true);
        push_json_field_number(output, "logical", self.metrics.logical, true);
        push_json_field_number(output, "control_flow", self.metrics.control_flow, true);
        push_json_field_number(output, "other", self.metrics.other, true);
        output.push_str("}}");
        output.push_str(",\"failures\":[");
        for (index, failure) in self.failures.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            failure.write_json(output);
        }
        output.push_str("]}");
    }
}

#[derive(Debug, Clone)]
struct FailureEvidence {
    case_index: Option<u64>,
    case_kind: &'static str,
    label: Option<&'static str>,
    a: Option<U256>,
    b: Option<U256>,
    revm_result: Option<U256>,
    rv64_result: Option<U256>,
    error: String,
}

impl FailureEvidence {
    fn write_json(&self, output: &mut String) {
        output.push('{');
        push_json_optional_number(output, "case_index", self.case_index, false);
        push_json_field_string(output, "case_kind", self.case_kind, true);
        push_json_optional_string(output, "label", self.label, true);
        push_json_optional_u256(output, "operand_a", self.a, true);
        push_json_optional_u256(output, "operand_b", self.b, true);
        push_json_optional_u256(output, "revm_result", self.revm_result, true);
        push_json_optional_u256(output, "rv64_result", self.rv64_result, true);
        push_json_field_string(output, "error", &self.error, true);
        output.push('}');
    }
}

pub fn run_benchmark(options: BenchOptions) -> BenchmarkReport {
    let operations = options
        .operations
        .iter()
        .copied()
        .map(|opcode| run_operation(opcode, options.random_cases_per_operation, options.seed))
        .collect();
    BenchmarkReport {
        options,
        operations,
    }
}

fn run_operation(opcode: Opcode, random_count: usize, seed: u64) -> OperationReport {
    let directed = directed_cases();
    let total = directed.len() as u64 + random_count as u64;
    let mut report = OperationReport {
        opcode,
        directed: directed.len() as u64,
        random: random_count as u64,
        passed: 0,
        failed: 0,
        machine_code_hash: None,
        routine_instruction_words: None,
        executed_cases: 0,
        metrics: InstructionMetrics::default(),
        min_instructions: None,
        max_instructions: None,
        failures: Vec::new(),
    };
    let mut runner = match Rv64Runner::new(opcode) {
        Ok(runner) => runner,
        Err(error) => {
            report.failed = total;
            report.failures.push(FailureEvidence {
                case_index: None,
                case_kind: "initialization",
                label: None,
                a: None,
                b: None,
                revm_result: None,
                rv64_result: None,
                error,
            });
            return report;
        }
    };
    report.machine_code_hash = Some(runner.machine_code_hash());
    report.routine_instruction_words = Some(runner.routine_instruction_words());

    for (index, case) in directed.into_iter().enumerate() {
        check_case(
            &mut report,
            &mut runner,
            index as u64,
            "directed",
            Some(case.label),
            case.a,
            case.b,
        );
    }
    // Start each operation's random generator at the same seed. In an ALL
    // run ADD and SUB therefore receive the same ordered operand pairs.
    let mut state = seed;
    for index in 0..random_count {
        let a = random_u256(&mut state);
        let b = random_u256(&mut state);
        let case_index = report.directed + index as u64;
        check_case(&mut report, &mut runner, case_index, "random", None, a, b);
    }
    report
}

fn check_case(
    report: &mut OperationReport,
    runner: &mut Rv64Runner,
    case_index: u64,
    case_kind: &'static str,
    label: Option<&'static str>,
    a: U256,
    b: U256,
) {
    // Run both independent paths for every case, even if one reports an
    // error, so an evidence record can include every result that was available.
    let (revm_result, revm_error) = match run_evm(report.opcode, a, b) {
        Ok(result) => (Some(result), None),
        Err(error) => (None, Some(error)),
    };
    let (rv64_execution, rv64_error) = match runner.run(a, b) {
        Ok(execution) => (Some(execution), None),
        Err(error) => (None, Some(error)),
    };
    if let Some(execution) = &rv64_execution {
        report.executed_cases += 1;
        report.metrics.add_assign(execution.metrics);
        report.min_instructions = Some(
            report
                .min_instructions
                .map_or(execution.metrics.total, |min| {
                    min.min(execution.metrics.total)
                }),
        );
        report.max_instructions = Some(
            report
                .max_instructions
                .map_or(execution.metrics.total, |max| {
                    max.max(execution.metrics.total)
                }),
        );
    }
    match (revm_result, rv64_execution) {
        (Some(expected), Some(actual)) if expected == actual.result => report.passed += 1,
        (revm_result, rv64_execution) => {
            report.failed += 1;
            let rv64_result = rv64_execution.map(|execution| execution.result);
            let error = match (&revm_result, &rv64_result) {
                (Some(_), Some(_)) => "differential result mismatch".to_owned(),
                _ => [revm_error, rv64_error]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("; "),
            };
            report.failures.push(FailureEvidence {
                case_index: Some(case_index),
                case_kind,
                label,
                a: Some(a),
                b: Some(b),
                revm_result,
                rv64_result,
                error,
            });
        }
    }
}

fn push_json_field_number(output: &mut String, name: &str, value: u64, comma: bool) {
    if comma {
        output.push(',');
    }
    write!(output, "\"{name}\":{value}").expect("writing JSON to String");
}

fn push_json_optional_number(output: &mut String, name: &str, value: Option<u64>, comma: bool) {
    if comma {
        output.push(',');
    }
    write!(output, "\"{name}\":").expect("writing JSON to String");
    match value {
        Some(value) => write!(output, "{value}").expect("writing JSON to String"),
        None => output.push_str("null"),
    }
}

fn push_json_optional_hex_u64(output: &mut String, name: &str, value: Option<u64>, comma: bool) {
    if comma {
        output.push(',');
    }
    write!(output, "\"{name}\":").expect("writing JSON to String");
    match value {
        Some(value) => push_json_string(output, &format!("fnv1a64:0x{value:016x}")),
        None => output.push_str("null"),
    }
}

fn push_json_field_string(output: &mut String, name: &str, value: &str, comma: bool) {
    if comma {
        output.push(',');
    }
    write!(output, "\"{name}\":").expect("writing JSON to String");
    push_json_string(output, value);
}

fn push_json_optional_string(output: &mut String, name: &str, value: Option<&str>, comma: bool) {
    if comma {
        output.push(',');
    }
    write!(output, "\"{name}\":").expect("writing JSON to String");
    match value {
        Some(value) => push_json_string(output, value),
        None => output.push_str("null"),
    }
}

fn push_json_optional_u256(output: &mut String, name: &str, value: Option<U256>, comma: bool) {
    if comma {
        output.push(',');
    }
    write!(output, "\"{name}\":").expect("writing JSON to String");
    match value {
        Some(value) => push_json_string(output, &hex(value)),
        None => output.push_str("null"),
    }
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                write!(output, "\\u{:04x}", character as u32).expect("writing JSON to String")
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::{BenchOptions, run_benchmark, run_guest_benchmark};
    use crate::Opcode;
    use ferreth_compiler::execution::DEFAULT_RANDOM_SEED;

    #[test]
    fn json_evidence_is_deterministic_and_metrics_are_complete() {
        let options = BenchOptions {
            operations: vec![Opcode::Add, Opcode::Sub],
            random_cases_per_operation: 2,
            seed: DEFAULT_RANDOM_SEED,
        };
        let first = run_benchmark(options.clone());
        let second = run_benchmark(options);
        assert!(first.passed());
        assert_eq!(first.to_json(), second.to_json());
        assert!(first.to_json().starts_with("{\"schema_version\":1,"));
        assert!(first.to_json().contains("\"name\":\"ADD\""));
        assert!(first.to_json().contains("\"name\":\"SUB\""));
        for operation in &first.operations {
            assert!(operation.metrics.total > 0);
            assert_eq!(operation.metrics.total, operation.metrics.category_sum());
            assert_eq!(operation.failed, 0);
        }
    }

    #[test]
    fn guest_benchmark_reports_observed_components_and_is_deterministic() {
        let first = run_guest_benchmark().expect("guest benchmark completes");
        let second = run_guest_benchmark().expect("repeated guest benchmark completes");
        assert_eq!(first, second);
        assert_eq!(first.to_json(), second.to_json());

        let opcode_totals = first
            .opcode_guest
            .iter()
            .map(|measurement| {
                let metrics = measurement.metrics;
                assert_eq!(
                    metrics.total(),
                    metrics.opcode_body() + metrics.guest_termination()
                );
                (
                    measurement.name,
                    metrics.opcode_body(),
                    metrics.guest_termination(),
                    metrics.total(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            opcode_totals,
            vec![
                ("PUSH1 0x05", 17, 9, 26),
                ("ADD 0x05 0x03", 42, 9, 51),
                ("SUB 0x05 0x03", 42, 9, 51),
                ("POP [0x00]", 3, 9, 12),
                ("DUP1 [0x00]", 14, 9, 23),
            ]
        );
        let program_totals = first
            .program_guest
            .iter()
            .map(|measurement| (measurement.bytecode, measurement.metrics.total()))
            .collect::<Vec<_>>();
        assert_eq!(
            program_totals,
            vec![
                ("6005", 19 + 7),
                ("60056003", 29 + 14),
                ("6005600301", 71 + 14),
                ("6005600303", 71 + 14),
                ("600560030100", 72 + 14),
                ("600550", 22 + 7),
                ("600580", 33 + 7),
                ("60058001", 75 + 7),
                ("6005600350", 32 + 14),
                ("60058050", 36 + 7),
            ]
        );
    }
}

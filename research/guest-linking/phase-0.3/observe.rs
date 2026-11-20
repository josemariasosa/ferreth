use alloy_primitives::U256;
use asm_rs::{Arch, Assembler};
use ferreth_compiler::{
    compiler::{CompiledRv64Program, compile_evm_bytecode, compile_push1_program},
    execution::{hex, random_u256},
    orchestrator::*,
    rv64::{
        executor::{IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{initialize_guest_state, read_guest_result, write_evm_word},
        memory::{ENTRY_PC, EVM_STACK_REGION, EXIT_PC, METADATA_REGION},
    },
};
use std::{fmt::Write as _, fs, path::Path};

fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn stack(values: &[U256]) -> String {
    values
        .iter()
        .copied()
        .map(hex)
        .collect::<Vec<_>>()
        .join(",")
}

fn metadata(executor: &mut RvemuExecutor) -> [u64; 3] {
    let mut raw = [0; 24];
    executor
        .read_memory(METADATA_REGION.start(), &mut raw)
        .unwrap();
    std::array::from_fn(|index| {
        u64::from_le_bytes(raw[index * 8..index * 8 + 8].try_into().unwrap())
    })
}

fn capture(
    out: &Path,
    id: &str,
    bytecode: &str,
    initial: &[U256],
    compiled: Option<&CompiledRv64Program>,
    observed: Result<OrchestratorResult, OrchestratorError>,
) {
    let observed = observed.unwrap_or_else(|error| panic!("{id}: orchestration failed: {error:?}"));
    let code = observed.machine_code();
    let assembly = observed.assembly();
    let result = observed.guest_result();
    let metrics = observed.instruction_metrics();
    let limit = u64::try_from(code.len() / 4).unwrap();
    assert_eq!(code.len() % 4, 0);
    assert_eq!(
        metrics.opcode_body() + metrics.guest_termination(),
        metrics.total()
    );

    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.base_address(ENTRY_PC);
    assembler.emit(assembly).unwrap();
    let assembled = assembler.finish().unwrap();
    assert_eq!(assembled.bytes(), code, "{id}: assembly reproduction");
    let mut publisher_pcs = Vec::new();
    let mut labels = String::new();
    for (name, pc) in assembled.labels() {
        let role = if name == "normal_exit" || name.ends_with("_normal_exit") {
            "normal"
        } else if name.ends_with("_stack_overflow_exit") {
            "overflow"
        } else if name.ends_with("_stack_underflow_exit") {
            "underflow"
        } else {
            ""
        };
        if !role.is_empty() {
            publisher_pcs.push(*pc);
        }
        writeln!(labels, "{name}\t{pc}\t{role}").unwrap();
    }
    publisher_pcs.sort_unstable();
    if let Some(compiled) = compiled {
        assert_eq!(compiled.assembly(), assembly);
        assert_eq!(compiled.machine_code(), code);
        assert_eq!(compiled.termination_entry_pcs(), publisher_pcs);
        assert_eq!(compiled.entry_pc(), ENTRY_PC);
        assert_eq!(compiled.exit_pc(), EXIT_PC);
        assert_eq!(compiled.instruction_limit(), limit);
        fs::write(
            out.join(format!("{id}.compiler.tsv")),
            format!(
                "{}\t{}\t{}\t{:?}\n",
                compiled.normal_termination_instructions(),
                compiled.exceptional_termination_instructions(),
                compiled.instruction_limit(),
                compiled.termination_entry_pcs()
            ),
        )
        .unwrap();
    }

    // Replay the exact bytes through public executor APIs to sample raw ABI state
    // around each publisher instruction. Counts above come from the ordinary API.
    let mut executor = RvemuExecutor::new().unwrap();
    executor.load_code(code).unwrap();
    executor
        .write_memory(EXIT_PC, &0x0010_0073u32.to_le_bytes())
        .unwrap();
    initialize_guest_state(&mut executor).unwrap();
    for (index, value) in initial.iter().copied().enumerate() {
        write_evm_word(&mut executor, index, value).unwrap();
    }
    let base = EVM_STACK_REGION.start();
    let initial_s1 = base + 32 * initial.len() as u64;
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, initial_s1);
    let initial_sp = executor.read_register(IntegerRegister::STACK_POINTER);
    assert_eq!(metadata(&mut executor), [0, 0, base]);
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        base
    );

    let body = executor
        .execute_until(ENTRY_PC, &publisher_pcs, limit)
        .unwrap();
    assert_eq!(body.executed_instructions, metrics.opcode_body());
    assert_eq!(metadata(&mut executor), [0, 0, base]);
    let final_s1 = result.final_s1();
    let status = result.status().as_u64();
    let exception = result.exception_code();
    assert_eq!(final_s1, base + 32 * result.stack_depth());
    assert!(final_s1 <= EVM_STACK_REGION.end());
    let mut trace = String::new();
    let mut pc = body.exit_pc;
    let termination = metrics.guest_termination();
    let mut raw = [0, 0, base];
    let mut total_replayed = body.executed_instructions;
    writeln!(trace, "0\t{pc}\t{}\t{}\t{}", raw[0], raw[1], raw[2]).unwrap();
    for step in 1..=termination {
        let next_pc = if step == termination { EXIT_PC } else { pc + 4 };
        let report = executor.execute(pc, next_pc, 1).unwrap();
        assert_eq!(report.executed_instructions, 1);
        assert_eq!(report.exit_pc, next_pc);
        total_replayed += report.executed_instructions;
        match step {
            3 => raw[2] = final_s1,
            4 if status == 1 => raw[1] = exception,
            5 if status == 2 => raw[1] = exception,
            _ if step == termination - 1 => raw[0] = status,
            _ => {}
        }
        let actual = metadata(&mut executor);
        assert_eq!(
            actual, raw,
            "{id}: metadata after publisher instruction {step}"
        );
        pc = report.exit_pc;
        writeln!(
            trace,
            "{step}\t{pc}\t{}\t{}\t{}",
            actual[0], actual[1], actual[2]
        )
        .unwrap();
        if step == termination {
            assert_eq!(&read_guest_result(&mut executor, &report).unwrap(), result);
        }
    }
    let final_sp = executor.read_register(IntegerRegister::STACK_POINTER);
    assert_eq!(initial_sp, final_sp);
    assert_eq!(total_replayed, metrics.total());

    fs::write(out.join(format!("{id}.s")), assembly).unwrap();
    fs::write(out.join(format!("{id}.bin")), code).unwrap();
    fs::write(out.join(format!("{id}.labels.tsv")), labels).unwrap();
    fs::write(out.join(format!("{id}.abi.tsv")), trace).unwrap();
    let mut record = String::new();
    writeln!(record, "{bytecode}\t{}\t{}\t{status}\t{exception}\t{}\t{final_s1}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        stack(initial), stack(result.stack()), initial_s1, result.stack_depth(),
        metrics.opcode_body(), termination, metrics.total(), body.exit_pc,
        initial_sp, final_sp, ENTRY_PC, EXIT_PC).unwrap();
    fs::write(out.join(format!("{id}.tsv")), record).unwrap();
}

fn ordinary(out: &Path, id: &str, input: &str, initial: &[U256]) {
    let code = bytes(input);
    let compiled = compile_evm_bytecode(&code)
        .unwrap_or_else(|error| panic!("{id}: compilation failed: {error:?}"));
    capture(
        out,
        id,
        input,
        initial,
        Some(&compiled),
        execute_guest_evm_bytecode_with_initial_stack(&code, initial),
    );
}

fn main() {
    let destination = std::env::args_os().nth(1).unwrap();
    let out = Path::new(&destination);
    fs::create_dir_all(out).unwrap();
    let operands = [U256::from(2), U256::from(3)];
    for (id, code, initial) in [
        ("rep_empty", "", &[][..]),
        ("rep_push", "6002", &[]),
        ("rep_push_stop", "600200", &[]),
        ("rep_two_push", "60026003", &[]),
        ("rep_push_add_stop", "600260030100", &[]),
        ("rep_add_stop", "0100", &[]),
        ("rep_stop", "00", &[]),
        ("rep_add", "01", &operands),
        ("rep_sub", "03", &operands),
    ] {
        ordinary(out, id, code, initial);
    }
    let mut seed = 42;
    let full: Vec<_> = (0..1024).map(|_| random_u256(&mut seed)).collect();
    let one = &full[..1];
    for (id, code, initial) in [
        ("exc_push_eof", "6002", &full[..]),
        ("exc_push_stop", "600200", &full),
        ("exc_dup_full", "80", &full),
        ("exc_dup_empty", "80", &[]),
        ("exc_pop_empty", "50", &[]),
        ("exc_add_empty", "01", &[]),
        ("exc_add_one", "01", one),
        ("eq_sub_empty", "03", &[]),
        ("eq_sub_one", "03", one),
    ] {
        ordinary(out, id, code, initial);
    }

    let sequence = compile_push1_program(&[2, 3]).expect("sequence compilation");
    let bytecode = compile_evm_bytecode(&bytes("60026003")).expect("bytecode compilation");
    fs::write(
        out.join("sequence_compiled_eq.txt"),
        format!("{}\n", sequence == bytecode),
    )
    .unwrap();
    capture(
        out,
        "eq_sequence",
        "60026003",
        &[],
        Some(&sequence),
        execute_guest_push1_sequence(&[2, 3]),
    );
    let full_sequence = compile_push1_program(&[2]).expect("sequence compilation");
    capture(
        out,
        "eq_sequence_full",
        "6002",
        &full,
        Some(&full_sequence),
        execute_guest_push1_sequence_with_initial_stack(&[2], &full),
    );

    capture(
        out,
        "eq_add",
        "01",
        &operands,
        None,
        execute_guest_add(operands[0], operands[1]),
    );
    capture(
        out,
        "eq_sub",
        "03",
        &operands,
        None,
        execute_guest_sub(operands[0], operands[1]),
    );
    for (suffix, initial) in [("empty", &[][..]), ("one", one)] {
        capture(
            out,
            &format!("iso_add_{suffix}"),
            "01",
            initial,
            None,
            execute_guest_add_with_initial_stack(initial),
        );
        capture(
            out,
            &format!("iso_sub_{suffix}"),
            "03",
            initial,
            None,
            execute_guest_sub_with_initial_stack(initial),
        );
    }

    let benchmark_operands = [U256::from(5), U256::from(3)];
    for (id, code, initial, result) in [
        ("bench_op_push", "6005", &[][..], execute_guest_push1(5)),
        (
            "bench_op_add",
            "01",
            &benchmark_operands[..],
            execute_guest_add(benchmark_operands[0], benchmark_operands[1]),
        ),
        (
            "bench_op_sub",
            "03",
            &benchmark_operands[..],
            execute_guest_sub(benchmark_operands[0], benchmark_operands[1]),
        ),
        ("bench_op_pop", "50", &[U256::ZERO][..], execute_guest_pop()),
        (
            "bench_op_dup",
            "80",
            &[U256::ZERO][..],
            execute_guest_dup1(),
        ),
    ] {
        capture(out, id, code, initial, None, result);
    }
    for code in [
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
    ] {
        ordinary(out, &format!("bench_program_{code}"), code, &[]);
    }
}

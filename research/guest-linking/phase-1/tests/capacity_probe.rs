use std::fmt::Write;

use asm_rs::{Arch, AsmError, Assembler, ResourceLimits};
use ferreth_compiler::rv64::memory::{validate_generated_code, ENTRY_PC, EXIT_PC};

#[allow(dead_code)]
mod symbolic_push_fixture {
    include!("support/symbolic_push1.rs");

    pub fn assemble(immediates: &[u8]) -> (String, asm_rs::AssemblyResult) {
        assemble_push_guest(immediates, false)
    }
}

#[test]
#[ignore = "bounded Phase 2.0 assembler and guest-capacity experiment"]
fn measure_symbolic_guest_capacity() {
    let defaults = ResourceLimits::default();
    println!("asm-rs 0.2.0 ResourceLimits defaults: {defaults:?}");

    let exact_labels = assemble_labeled_nops(defaults.max_labels);
    assert_eq!(exact_labels.labels().len(), defaults.max_labels);
    assert_eq!(exact_labels.len(), 4);
    println!(
        "default label boundary: {} labels assembled to {} bytes",
        exact_labels.labels().len(),
        exact_labels.len()
    );

    let too_many_labels = labeled_nop_source(defaults.max_labels + 1);
    let mut assembler = Assembler::new(Arch::Rv64);
    let error = assembler
        .base_address(ENTRY_PC)
        .emit(&too_many_labels)
        .unwrap_err();
    assert!(matches!(
        error,
        AsmError::ResourceLimitExceeded {
            ref resource,
            limit,
        }
            if resource == "labels" && limit == defaults.max_labels
    ));
    println!(
        "default label rejection: source={} bytes, {error:?}",
        too_many_labels.len()
    );

    let mut limits = defaults;
    limits.max_source_bytes = 3;
    assert_resource_limit("addi zero, zero, 0\n", limits, "source bytes", 3);

    let mut limits = defaults;
    limits.max_statements = 1;
    assert_resource_limit(
        "addi zero, zero, 0\naddi zero, zero, 0\n",
        limits,
        "statements",
        1,
    );

    let mut limits = defaults;
    limits.max_output_bytes = 4;
    assert_resource_limit(
        "addi zero, zero, 0\naddi zero, zero, 0\n",
        limits,
        "output bytes",
        4,
    );

    for pushes in [1, 47, 23_829, 23_830] {
        let immediates = vec![0; pushes];
        let (source, result) = symbolic_push_fixture::assemble(&immediates);
        let ferreth_layout = validate_generated_code(ENTRY_PC, result.len());
        assert_eq!(result.base_address(), ENTRY_PC);
        assert_eq!(result.labels().len(), pushes * 3 + 2);
        if pushes == 23_829 {
            assert_eq!(result.len(), 1_048_536);
            assert_eq!(ferreth_layout, Ok(()));
        } else if pushes == 23_830 {
            assert_eq!(result.len(), 1_048_580);
            assert!(ferreth_layout.is_err());
        } else {
            assert_eq!(ferreth_layout, Ok(()));
        }
        println!(
            "symbolic PUSH1 x{pushes}: source={} bytes, labels={}, output={} bytes, Ferreth_layout={ferreth_layout:?}",
            source.len(),
            result.labels().len(),
            result.len()
        );
    }

    let max_pop_count = (defaults.max_labels - 3) / 3;
    let (source, result) = assemble_symbolic_pops(max_pop_count);
    let result = result.unwrap();
    assert_eq!(result.labels().len(), 3 * max_pop_count + 3);
    assert_eq!(result.len(), 16 * max_pop_count + 92);
    assert_eq!(result.labels().len(), 99_999);
    assert_eq!(result.len(), 533_404);
    assert_eq!(validate_generated_code(ENTRY_PC, result.len()), Ok(()));
    println!(
        "symbolic POP x{max_pop_count}: source={} bytes, labels={}, output={} bytes, Ferreth_layout=Ok(())",
        source.len(),
        result.labels().len(),
        result.len()
    );

    let (too_many_pop_source, too_many_pop_result) = assemble_symbolic_pops(max_pop_count + 1);
    assert!(too_many_pop_result.is_err());
    let error = too_many_pop_result.unwrap_err();
    assert!(matches!(
        error,
        AsmError::ResourceLimitExceeded {
            ref resource,
            limit,
        }
            if resource == "labels" && limit == defaults.max_labels
    ));
    let projected_bytes = 16 * (max_pop_count + 1) + 92;
    assert!(projected_bytes < (EXIT_PC - ENTRY_PC) as usize);
    println!(
        "symbolic POP x{}: source={} bytes, projected output={} bytes, rejected before code bound: {error:?}",
        max_pop_count + 1,
        too_many_pop_source.len(),
        projected_bytes
    );
}

fn labeled_nop_source(labels: usize) -> String {
    let mut source = String::new();
    for index in 0..labels {
        writeln!(source, "label_{index}:").unwrap();
    }
    source.push_str("addi zero, zero, 0\n");
    source
}

fn assert_resource_limit(
    source: &str,
    limits: ResourceLimits,
    expected_resource: &str,
    expected_limit: usize,
) {
    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.limits(limits).base_address(ENTRY_PC);
    let error = match assembler.emit(source) {
        Err(error) => error,
        Ok(_) => panic!("assembler did not enforce {expected_resource} limit"),
    };
    assert!(matches!(
        error,
        AsmError::ResourceLimitExceeded {
            ref resource,
            limit
        } if resource == expected_resource && limit == expected_limit
    ));
    println!("configured limit: {error:?}");
}

fn assemble_labeled_nops(labels: usize) -> asm_rs::AssemblyResult {
    let source = labeled_nop_source(labels);
    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.base_address(ENTRY_PC).emit(&source).unwrap();
    assembler.finish().unwrap()
}

fn assemble_symbolic_pops(count: usize) -> (String, Result<asm_rs::AssemblyResult, AsmError>) {
    let mut source = String::new();
    for index in 0..count {
        writeln!(
            source,
            r#"guest_pop_{index}:
    bgeu s0, s1, pop_underflow_{index}
    addi s1, s1, -32
    jal zero, pop_continue_{index}
pop_underflow_{index}:
    jal zero, binary_stack_underflow_exit
pop_continue_{index}:"#
        )
        .unwrap();
    }
    source.push_str(
        r#"normal_exit:
    addi t1, s0, -2048
    addi t1, t1, -2048
    sd s1, 16(t1)
    sd zero, 8(t1)
    addi t0, zero, 1
    sd t0, 0(t1)
    jalr zero, -4(t1)
push1_stack_overflow_exit:
    addi t1, s0, -2048
    addi t1, t1, -2048
    sd s1, 16(t1)
    addi t0, zero, 1
    sd t0, 8(t1)
    addi t0, zero, 2
    sd t0, 0(t1)
    jalr zero, -4(t1)
binary_stack_underflow_exit:
    addi t1, s0, -2048
    addi t1, t1, -2048
    sd s1, 16(t1)
    addi t0, zero, 2
    sd t0, 8(t1)
    addi t0, zero, 2
    sd t0, 0(t1)
    jalr zero, -4(t1)
"#,
    );

    let mut assembler = Assembler::new(Arch::Rv64);
    let result = match assembler.base_address(ENTRY_PC).emit(&source) {
        Ok(_) => assembler.finish(),
        Err(error) => Err(error),
    };
    (source, result)
}

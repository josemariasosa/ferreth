use std::{collections::BTreeSet, fmt::Write, ops::Range};

use asm_rs::{Arch, Assembler, AssemblyResult};
use ferreth_compiler::{
    compiler::compile_evm_bytecode,
    decode,
    execution::{random_u256, Rv64Runner},
    rv64::{
        executor::{IntegerRegister, Rv64Executor, RvemuExecutor},
        guest::{
            initialize_guest_state, read_guest_result, GuestExecutionStatus, GuestMetadata,
            GUEST_METADATA_ENCODED_SIZE,
        },
        memory::{
            validate_generated_code, ENTRY_PC, EVM_STACK_CAPACITY, EVM_STACK_REGION, EVM_WORD_SIZE,
            EXIT_PC, METADATA_REGION,
        },
    },
    Instruction, Opcode,
};

const NORMAL: &str = "normal_exit";
const OVERFLOW: &str = "push1_stack_overflow_exit";
const UNDERFLOW: &str = "binary_stack_underflow_exit";
type Word = [u64; 4];

fn entry_name(op: Instruction, index: usize) -> String {
    match op {
        Instruction::Stop => "guest_stop".to_owned(),
        Instruction::Push(_) => format!("guest_push1_{index}"),
        Instruction::Pop => format!("guest_pop_{index}"),
        Instruction::Dup1 => format!("guest_dup1_{index}"),
        Instruction::Add | Instruction::Sub => format!("guest_arithmetic_{index}"),
    }
}

fn core_source(opcode: Opcode) -> String {
    opcode
        .assembly()
        .lines()
        .filter(|line| !line.trim_start().starts_with('.'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assemble(source: &str) -> AssemblyResult {
    let mut assembler = Assembler::new(Arch::Rv64);
    assembler.base_address(ENTRY_PC).emit(source).unwrap();
    assembler.finish().unwrap()
}

fn experimental_source(ops: &[Instruction]) -> String {
    let mut source = String::new();
    for (index, &op) in ops.iter().enumerate() {
        let entry = entry_name(op, index);
        let next = format!("continue_{index}");
        let under = format!("underflow_{index}");
        let over = format!("overflow_{index}");
        writeln!(source, "{entry}:").unwrap();
        match op {
            Instruction::Stop => {
                writeln!(source, "    jal zero, {NORMAL}").unwrap();
                break;
            }
            Instruction::Push(value) => {
                let immediate = value.to::<u8>();
                writeln!(
                    source,
                    r#"    lui t0, 8
    add t0, s0, t0
    bgeu s1, t0, {over}
    addi t0, zero, {immediate}
    sd t0, 0(s1)
    sd zero, 8(s1)
    sd zero, 16(s1)
    sd zero, 24(s1)
    addi s1, s1, 32
    jal zero, {next}
{over}:
    jal zero, {OVERFLOW}"#
                )
                .unwrap();
            }
            Instruction::Pop => writeln!(
                source,
                r#"    bgeu s0, s1, {under}
    addi s1, s1, -32
    jal zero, {next}
{under}:
    jal zero, {UNDERFLOW}"#
            )
            .unwrap(),
            Instruction::Dup1 => writeln!(
                source,
                r#"    bgeu s0, s1, {under}
    lui t0, 8
    add t0, s0, t0
    bgeu s1, t0, {over}
    ld t1, -32(s1)
    ld t2, -24(s1)
    ld t3, -16(s1)
    ld t4, -8(s1)
    sd t1, 0(s1)
    sd t2, 8(s1)
    sd t3, 16(s1)
    sd t4, 24(s1)
    addi s1, s1, 32
    jal zero, {next}
{under}:
    jal zero, {UNDERFLOW}
{over}:
    jal zero, {OVERFLOW}"#
            )
            .unwrap(),
            Instruction::Add | Instruction::Sub => {
                let (opcode, name) = if op == Instruction::Add {
                    (Opcode::Add, "evm_add")
                } else {
                    (Opcode::Sub, "evm_sub")
                };
                let core = core_source(opcode);
                let body = core.strip_prefix(&format!("{name}:\n")).unwrap();
                writeln!(
                    source,
                    r#"    addi t0, s0, 64
    bltu s1, t0, {under}
    addi a0, s1, -32
    addi a1, s1, -64
    addi a2, s1, -64
    jal ra, {name}_{index}
return_{index}:
    addi s1, s1, -32
    jal zero, {next}
{under}:
    jal zero, {UNDERFLOW}
{name}_{index}:
{body}"#
                )
                .unwrap();
            }
        }
        writeln!(source, "{next}:").unwrap();
    }
    writeln!(
        source,
        r#"{NORMAL}:
    addi t1, s0, -2048
    addi t1, t1, -2048
    sd s1, 16(t1)
    sd zero, 8(t1)
    addi t0, zero, 1
    sd t0, 0(t1)
    jalr zero, -4(t1)"#
    )
    .unwrap();
    for (name, exception) in [(OVERFLOW, 1), (UNDERFLOW, 2)] {
        writeln!(
            source,
            r#"{name}:
    addi t1, s0, -2048
    addi t1, t1, -2048
    sd s1, 16(t1)
    addi t0, zero, {exception}
    sd t0, 8(t1)
    addi t0, zero, 2
    sd t0, 0(t1)
    jalr zero, -4(t1)"#
        )
        .unwrap();
    }
    source
}

#[derive(Debug, PartialEq, Eq)]
enum Transfer {
    Branch(u32, u64),
    Jal(u32, u64),
    Jalr(u32),
}

// Decode final words independently of the assembler's relocation calculations.
fn decode_transfer(pc: u64, word: u32) -> Option<Transfer> {
    let target = |immediate, bits: u32| {
        let shift = 32 - bits;
        let delta = ((immediate << shift) as i32) >> shift;
        pc.checked_add_signed(i64::from(delta)).unwrap()
    };
    match word & 0x7f {
        0x63 => {
            let immediate = ((word >> 31) << 12)
                | (((word >> 25) & 0x3f) << 5)
                | (((word >> 8) & 0xf) << 1)
                | (((word >> 7) & 1) << 11);
            Some(Transfer::Branch(word & 0x01ff_f07f, target(immediate, 13)))
        }
        0x6f => {
            let immediate = ((word >> 31) << 20)
                | (((word >> 21) & 0x3ff) << 1)
                | (((word >> 20) & 1) << 11)
                | (word & 0xff000);
            Some(Transfer::Jal((word >> 7) & 0x1f, target(immediate, 21)))
        }
        0x67 => Some(Transfer::Jalr(word)),
        _ => None,
    }
}

fn bytes_at(code: &[u8], range: Range<u64>) -> &[u8] {
    assert!(range.start.is_multiple_of(4) && range.end.is_multiple_of(4));
    &code[(range.start - ENTRY_PC) as usize..(range.end - ENTRY_PC) as usize]
}

fn check_transfers(code: &[u8], range: Range<u64>, expected: &[Transfer]) -> Vec<u64> {
    let start = range.start;
    let mut sites = Vec::new();
    let mut actual = Vec::new();
    for (index, &word) in bytes_at(code, range).as_chunks::<4>().0.iter().enumerate() {
        let pc = start + index as u64 * 4;
        let word = u32::from_le_bytes(word);
        if let Some(transfer) = decode_transfer(pc, word) {
            if let Transfer::Branch(_, target) | Transfer::Jal(_, target) = transfer {
                bytes_at(code, target..target + 4);
            }
            println!("  {pc:08x} {word:08x} {transfer:?}");
            sites.push(pc);
            actual.push(transfer);
        }
    }
    assert_eq!(actual, expected, "transfers in fragment at {start:#010x}");
    sites
}

struct Layout {
    publishers: [u64; 3],
    // (EVM instruction ordinal, core entry, return to the wrapper)
    calls: Vec<(usize, u64, u64)>,
    // (EVM instruction ordinal, trampoline, publisher)
    traps: Vec<(usize, u64, u64)>,
}

fn check_label_inventory(labels: &[(String, u64)], expected: &BTreeSet<String>) {
    let actual: BTreeSet<_> = labels.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(actual.len(), labels.len(), "duplicate label names");
    assert_eq!(
        actual,
        expected.iter().map(String::as_str).collect(),
        "unexpected or omitted labels"
    );
}

fn check_layout(image: &AssemblyResult, ops: &[Instruction], experimental: bool) -> Layout {
    let code = image.bytes();
    validate_generated_code(ENTRY_PC, code.len()).unwrap();
    assert_eq!(image.base_address(), ENTRY_PC);
    let mut names = BTreeSet::new();
    let mut sites = BTreeSet::new();
    let mut label = |name: &str| {
        assert!(names.insert(name.to_owned()), "duplicate lookup for {name}");
        image.label_address(name).unwrap()
    };
    let normal = label(NORMAL);
    let overflow = if experimental
        || ops
            .iter()
            .any(|op| matches!(op, Instruction::Push(_) | Instruction::Dup1))
    {
        label(OVERFLOW)
    } else {
        0
    };
    let underflow = if experimental
        || ops.iter().any(|op| {
            matches!(
                op,
                Instruction::Pop | Instruction::Dup1 | Instruction::Add | Instruction::Sub
            )
        }) {
        label(UNDERFLOW)
    } else {
        0
    };
    let mut expected_pc = ENTRY_PC;
    let entries: Vec<_> = ops
        .iter()
        .enumerate()
        .map(|(i, &op)| {
            let entry = label(&entry_name(op, i));
            assert_eq!(entry, expected_pc, "fragment {i} ({op:?})");
            expected_pc += match op {
                Instruction::Stop => 4,
                Instruction::Push(_) => 44,
                Instruction::Pop => 16,
                Instruction::Dup1 => 64,
                Instruction::Add | Instruction::Sub => 172,
            };
            entry
        })
        .collect();
    let mut layout = Layout {
        publishers: [normal, overflow, underflow],
        calls: vec![],
        traps: vec![],
    };

    use Transfer::{Branch, Jal, Jalr};
    const EMPTY: u32 = 0x0094_7063; // BGEU s0, s1
    const FULL: u32 = 0x0054_f063; // BGEU s1, t0
    const BELOW_TWO: u32 = 0x0054_e063; // BLTU s1, t0
    for (i, (&op, &entry)) in ops.iter().zip(&entries).enumerate() {
        let next = entries.get(i + 1).copied().unwrap_or(normal);
        assert!(sites.insert(entry));
        if experimental && op != Instruction::Stop {
            assert_eq!(label(&format!("continue_{i}")), next);
        }
        let mut trap = |pc, publisher| {
            assert!(sites.insert(pc));
            layout.traps.push((i, pc, publisher));
            Jal(0, publisher)
        };
        match op {
            Instruction::Push(_) => {
                let over = if experimental {
                    label(&format!("overflow_{i}"))
                } else {
                    label(&format!("push1_overflow_{i}"))
                };
                if !experimental {
                    assert_eq!(label(&format!("push1_continue_{i}")), next);
                }
                let edges = [Branch(FULL, over), Jal(0, next), trap(over, overflow)];
                assert_eq!(check_transfers(code, entry..next, &edges)[2], over);
            }
            Instruction::Pop => {
                let name = if experimental {
                    format!("underflow_{i}")
                } else {
                    format!("pop_underflow_{i}")
                };
                let under = label(&name);
                let edges = [Branch(EMPTY, under), Jal(0, next), trap(under, underflow)];
                assert_eq!(check_transfers(code, entry..next, &edges)[2], under);
            }
            Instruction::Dup1 => {
                let (under_name, over_name) = if experimental {
                    (format!("underflow_{i}"), format!("overflow_{i}"))
                } else {
                    (format!("dup1_underflow_{i}"), format!("dup1_overflow_{i}"))
                };
                let under = label(&under_name);
                let over = label(&over_name);
                let edges = [
                    Branch(EMPTY, under),
                    Branch(FULL, over),
                    Jal(0, next),
                    trap(under, underflow),
                    trap(over, overflow),
                ];
                assert_eq!(
                    check_transfers(code, entry..next, &edges)[3..],
                    [under, over]
                );
            }
            Instruction::Add | Instruction::Sub => {
                let (opcode, name, expected_hash) = if op == Instruction::Add {
                    (Opcode::Add, "add", 0xf1ff_f32f_f479_62f3)
                } else {
                    (Opcode::Sub, "sub", 0x9f00_8f32_0eeb_7a7b)
                };
                let under_name = if experimental {
                    format!("underflow_{i}")
                } else {
                    format!("arith_underflow_{i}")
                };
                let under = label(&under_name);
                let core = if experimental {
                    label(&format!("evm_{name}_{i}"))
                } else {
                    label(&format!("arith_core_{i}"))
                };
                let edges = [
                    Branch(BELOW_TWO, under),
                    Jal(1, core),
                    Jal(0, next),
                    trap(under, underflow),
                ];
                let jump_sites = check_transfers(code, entry..core, &edges);
                assert_eq!(jump_sites[3], under);
                let return_pc = jump_sites[1] + 4;
                let update = if experimental {
                    format!("return_{i}")
                } else {
                    format!("arith_update_{i}")
                };
                assert_eq!(label(&update), return_pc);
                assert_eq!(
                    bytes_at(code, return_pc..return_pc + 4),
                    0xfe04_8493_u32.to_le_bytes()
                );
                assert!(sites.insert(core) && sites.insert(return_pc));
                layout.calls.push((i, core, return_pc));

                let standalone = assemble(&core_source(opcode));
                let embedded = bytes_at(code, core..next);
                assert_eq!(embedded, standalone.bytes());
                assert_eq!(embedded.len(), 136);
                assert_eq!(fnv1a(embedded), expected_hash);
                assert_eq!(
                    check_transfers(code, core..next, &[Jalr(0x0000_8067)]),
                    [next - 4]
                );
                println!("  {name} core {core:#010x}, return {return_pc:#010x}, fnv1a64 {expected_hash:#018x}");
            }
            Instruction::Stop => {
                assert_eq!(
                    check_transfers(code, entry..next, &[Jal(0, normal)]),
                    [entry]
                );
            }
        }
    }
    let publishers = if experimental {
        [
            (NORMAL, normal),
            (OVERFLOW, overflow),
            (UNDERFLOW, underflow),
        ]
    } else {
        [
            (NORMAL, normal),
            (UNDERFLOW, underflow),
            (OVERFLOW, overflow),
        ]
    };
    for (name, pc) in publishers.into_iter().filter(|&(_, pc)| pc != 0) {
        assert_eq!(pc, expected_pc, "{name} publisher PC");
        assert!(sites.insert(pc), "duplicate publisher site for {name}");
        let prefix = [0x8004_0313_u32, 0x8003_0313, 0x0093_3823];
        let suffix: &[u32] = match name {
            NORMAL => &[0x0003_3423, 0x0010_0293, 0x0053_3023, 0xffc3_0067],
            OVERFLOW => &[
                0x0010_0293,
                0x0053_3423,
                0x0020_0293,
                0x0053_3023,
                0xffc3_0067,
            ],
            UNDERFLOW => &[
                0x0020_0293,
                0x0053_3423,
                0x0020_0293,
                0x0053_3023,
                0xffc3_0067,
            ],
            _ => unreachable!(),
        };
        let words: Vec<_> = prefix
            .iter()
            .chain(suffix)
            .flat_map(|word| word.to_le_bytes())
            .collect();
        expected_pc += words.len() as u64;
        let range = pc..expected_pc;
        assert_eq!(
            bytes_at(code, range.clone()),
            words,
            "{name} publisher bytes"
        );
        assert_eq!(
            check_transfers(code, range, &[Jalr(0xffc3_0067)]),
            [expected_pc - 4],
            "{name} terminal JALR site"
        );
    }
    assert_eq!(expected_pc, ENTRY_PC + code.len() as u64, "publisher tail");
    check_label_inventory(image.labels(), &names);
    assert_eq!(EVM_STACK_REGION.start() - 2048 - 2048 - 4, EXIT_PC);
    println!(
        "  publishers {:08x?}, {} labels",
        layout.publishers,
        names.len()
    );
    layout
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn metadata(executor: &mut RvemuExecutor) -> GuestMetadata {
    let mut bytes = [0; GUEST_METADATA_ENCODED_SIZE];
    executor
        .read_memory(METADATA_REGION.start(), &mut bytes)
        .unwrap();
    GuestMetadata::decode(&bytes).unwrap()
}

fn stack_pointer(depth: usize) -> u64 {
    EVM_STACK_REGION.start() + depth as u64 * EVM_WORD_SIZE
}

fn execute(
    code: &[u8],
    layout: &Layout,
    initial: &[Word],
    expected: &[Word],
    fault: Option<(usize, usize)>,
) -> (u64, u64) {
    use GuestExecutionStatus::{GuestException, Normal, Running};

    let mut executor = RvemuExecutor::new().unwrap();
    executor.load_code(code).unwrap();
    executor
        .write_memory(EXIT_PC, &0x0010_0073_u32.to_le_bytes())
        .unwrap();
    initialize_guest_state(&mut executor).unwrap();
    for (index, limbs) in initial.iter().enumerate() {
        let bytes = limbs.map(u64::to_le_bytes);
        executor
            .write_memory(stack_pointer(index), bytes.as_flattened())
            .unwrap();
    }
    let initial_s1 = stack_pointer(initial.len());
    let running = GuestMetadata::new(Running, 0, initial_s1);
    executor.write_register(IntegerRegister::EVM_STACK_NEXT_FREE, initial_s1);
    executor
        .write_memory(METADATA_REGION.start(), &running.encode())
        .unwrap();

    let guard = [0xa5; EVM_WORD_SIZE as usize];
    executor
        .write_memory(EVM_STACK_REGION.end(), &guard)
        .unwrap();
    let native_sp = executor.read_register(IntegerRegister::STACK_POINTER);
    let (fault_index, exception_code) = fault.unwrap_or((usize::MAX, 0));
    let publisher = layout.publishers[exception_code];
    let mut stops = layout.publishers.to_vec();
    stops.push(EXIT_PC);
    stops.extend(layout.traps.iter().map(|&(_, pc, _)| pc));
    for &(_, core, return_pc) in &layout.calls {
        stops.extend([core, return_pc]);
    }
    let mut pc = ENTRY_PC;
    let mut body_count = 0;
    let budget = code.len() as u64 / 4;
    let mut run_to = |executor: &mut RvemuExecutor, boundary| {
        let active: Vec<_> = stops
            .iter()
            .copied()
            .filter(|&stop| stop != 0 && stop != pc)
            .collect();
        let report = executor
            .execute_until(pc, &active, budget - body_count)
            .unwrap();
        assert_eq!(report.exit_pc, boundary, "checkpoint after {pc:#010x}");
        assert_eq!(metadata(executor), running);
        println!(
            "  runtime checkpoint {boundary:#010x}, +{} RV64",
            report.executed_instructions
        );
        body_count += report.executed_instructions;
        pc = report.exit_pc;
        report.executed_instructions
    };
    for &(_, core, return_pc) in layout
        .calls
        .iter()
        .take_while(|&&(i, _, _)| i < fault_index)
    {
        run_to(&mut executor, core);
        let s1 = executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE);
        assert_eq!(
            executor.read_register(IntegerRegister::RETURN_ADDRESS),
            return_pc
        );
        for (register, address) in [(10, s1 - 32), (11, s1 - 64), (12, s1 - 64)] {
            assert_eq!(
                executor.read_register(IntegerRegister::new(register).unwrap()),
                address
            );
        }
        assert_eq!(run_to(&mut executor, return_pc), 34);
        assert_eq!(
            executor.read_register(IntegerRegister::RETURN_ADDRESS),
            return_pc
        );
        assert_eq!(
            executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
            s1
        );
    }
    if fault.is_some() {
        let &(_, trampoline, _) = layout
            .traps
            .iter()
            .find(|&&(i, _, target)| i == fault_index && target == publisher)
            .unwrap();
        run_to(&mut executor, trampoline);
        assert_eq!(run_to(&mut executor, publisher), 1);
    } else {
        run_to(&mut executor, publisher);
    }

    let final_s1 = stack_pointer(expected.len());
    let status = if fault.is_some() {
        GuestException
    } else {
        Normal
    };
    let terminal = GuestMetadata::new(status, exception_code as u64, final_s1);
    let publisher_count = if fault.is_some() { 8 } else { 7 };
    let status_store = publisher + (publisher_count - 2) * 4;
    for (boundary, expected_metadata) in [
        (publisher + 12, GuestMetadata::new(Running, 0, final_s1)),
        (
            status_store,
            GuestMetadata::new(Running, exception_code as u64, final_s1),
        ),
        (status_store + 4, terminal),
    ] {
        let count = (boundary - pc) / 4;
        let report = executor.execute(pc, boundary, count).unwrap();
        assert_eq!(report.executed_instructions, count);
        assert_eq!(metadata(&mut executor), expected_metadata);
        println!("  publication {boundary:#010x} {expected_metadata:?}");
        pc = report.exit_pc;
    }
    let exit = executor.execute(pc, EXIT_PC, 1).unwrap();
    assert_eq!(exit.executed_instructions, 1);
    let result = read_guest_result(&mut executor, &exit).unwrap();
    assert_eq!(result.metadata().metadata(), terminal);
    assert_eq!(result.stack_depth(), expected.len() as u64);
    assert!(result
        .stack()
        .iter()
        .map(|word| *word.as_limbs())
        .eq(expected.iter().copied()));
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_BASE),
        EVM_STACK_REGION.start()
    );
    assert_eq!(
        executor.read_register(IntegerRegister::EVM_STACK_NEXT_FREE),
        final_s1
    );
    assert_eq!(
        executor.read_register(IntegerRegister::STACK_POINTER),
        native_sp
    );
    let mut actual_guard = [0; EVM_WORD_SIZE as usize];
    executor
        .read_memory(EVM_STACK_REGION.end(), &mut actual_guard)
        .unwrap();
    assert_eq!(actual_guard, guard);
    println!("  result {terminal:?}, depth {}, sentinel {:#010x}, body {body_count}, publisher {publisher_count}, total {}",
        result.stack_depth(), exit.exit_pc, body_count + publisher_count);
    (body_count, publisher_count)
}

fn compare(bytecode: &[u8], initial: &[Word], expected: &[Word], fault: Option<(usize, usize)>) {
    let hex = |bytes: &[u8]| {
        bytes.iter().fold(String::new(), |mut out, byte| {
            write!(out, "{byte:02x}").unwrap();
            out
        })
    };
    println!(
        "specimen {} (initial depth {})",
        hex(bytecode),
        initial.len()
    );
    let ops = decode(bytecode).unwrap();
    let symbolic = assemble(&experimental_source(&ops));
    let compiled = compile_evm_bytecode(bytecode).unwrap();
    let production = assemble(compiled.assembly());
    assert_eq!(production.bytes(), compiled.machine_code());
    assert_eq!(
        (compiled.entry_pc(), compiled.exit_pc()),
        (ENTRY_PC, EXIT_PC)
    );

    println!("symbolic:");
    let symbolic_layout = check_layout(&symbolic, &ops, true);
    let symbolic_counts = execute(symbolic.bytes(), &symbolic_layout, initial, expected, fault);
    println!("production:");
    let production_layout = check_layout(&production, &ops, false);
    let [normal, overflow, underflow] = production_layout.publishers;
    let published: Vec<_> = [normal, underflow, overflow]
        .into_iter()
        .filter(|&pc| pc != 0)
        .collect();
    assert_eq!(compiled.termination_entry_pcs(), published);
    let production_counts = execute(
        compiled.machine_code(),
        &production_layout,
        initial,
        expected,
        fault,
    );
    assert_eq!(symbolic_counts, production_counts);

    let same_bytes = symbolic.bytes() == compiled.machine_code();
    println!(
        "comparison bytes {}/{}, words {}/{}, equal bytes {same_bytes}, counts {symbolic_counts:?}/{production_counts:?}",
        symbolic.len(), production.len(), symbolic.len() / 4, production.len() / 4
    );
    println!("symbolic hex {}", hex(symbolic.bytes()));
    if !same_bytes {
        println!("production hex {}", hex(compiled.machine_code()));
    }
}

const fn word(low: u64) -> Word {
    [low, 0, 0, 0]
}

#[test]
fn elementary_mixed_programs() {
    compare(&[0x60, 2, 0x60, 3, 1, 0], &[], &[word(5)], None);
    compare(&[0x60, 2, 0x60, 3, 3, 0], &[], &[word(1)], None);
    compare(&[0x60, 2, 0x50, 0], &[], &[], None);
    compare(&[0x60, 2, 0x80, 0], &[], &[word(2), word(2)], None);
    let wide = [0x1234, 0x5678, u64::MAX, 42];
    compare(&[0x80, 0x50, 0x80], &[wide], &[wide, wide], None);
}

#[test]
fn repeated_fragments_at_eof_and_stop() {
    let mut program = vec![
        0x60, 2, 0x60, 3, 1, 0x80, 0x60, 7, 3, 0x50, 0x60, 9, 1, 0x80, 0x50, 0x60, 8, 3,
    ];
    let expected = [[u64::MAX - 5, u64::MAX, u64::MAX, u64::MAX]];
    compare(&program, &[], &expected, None);
    program.push(0);
    compare(&program, &[], &expected, None);
}

#[test]
fn underflow_and_overflow_publishers() {
    for bytecode in [[1, 0], [0x50, 0], [0x80, 0]] {
        compare(&bytecode, &[], &[], Some((0, 2)));
    }
    compare(&[0x60, 2, 3, 0], &[], &[word(2)], Some((1, 2)));
    compare(&[0x60, 2, 1], &[], &[word(2)], Some((1, 2)));
    let mut seed = 42;
    let initial: Vec<_> = (0..EVM_STACK_CAPACITY)
        .map(|_| *random_u256(&mut seed).as_limbs())
        .collect();
    compare(&[0x80, 0], &initial, &initial, Some((0, 1)));
    compare(&[0x60, 2, 0x80, 0], &initial, &initial, Some((0, 1)));
    let mut fills_stack = initial[..initial.len() - 1].to_vec();
    fills_stack.push(*fills_stack.last().unwrap());
    compare(
        &[0x80, 0x80, 0],
        &initial[..initial.len() - 1],
        &fills_stack,
        Some((1, 1)),
    );
}

#[test]
fn arithmetic_across_limbs_and_standalone_core_hashes() {
    for (opcode, hash) in [
        (Opcode::Add, 0xf1ff_f32f_f479_62f3),
        (Opcode::Sub, 0x9f00_8f32_0eeb_7a7b),
    ] {
        let runner = Rv64Runner::new(opcode).unwrap();
        assert_eq!(runner.routine_instruction_words(), 34);
        assert_eq!(runner.machine_code_hash(), hash);
        assert_eq!(fnv1a(assemble(&core_source(opcode)).bytes()), hash);
        println!(
            "standalone {}: 34 words, fnv1a64 {hash:#018x}",
            opcode.name()
        );
    }
    compare(&[1], &[[u64::MAX; 4], word(1)], &[word(0)], None);
    compare(
        &[3],
        &[word(1), [0, 0, 0, 1]],
        &[[u64::MAX, u64::MAX, u64::MAX, 0]],
        None,
    );
}

#[test]
fn normal_only_and_overflow_only_production_inventories() {
    compare(&[0x00], &[], &[], None);
    compare(&[0x60, 2], &[], &[word(2)], None);
}

#[test]
#[should_panic(expected = "push1_stack_overflow_exit publisher bytes")]
fn experimental_layout_rejects_a_wrong_unreachable_publisher_body() {
    let ops = [Instruction::Stop];
    let source = experimental_source(&ops);
    check_layout(&assemble(&source), &ops, true);
    let mutated = source.replacen(
        "addi t0, zero, 1\n    sd t0, 8(t1)",
        "addi t0, zero, 2\n    sd t0, 8(t1)",
        1,
    );
    check_layout(&assemble(&mutated), &ops, true);
}

#[test]
#[should_panic(expected = "unexpected or omitted labels")]
fn production_layout_rejects_an_unexpected_label_alias() {
    let bytecode = [0x00];
    let compiled = compile_evm_bytecode(&bytecode).unwrap();
    let ops = decode(&bytecode).unwrap();
    check_layout(&assemble(compiled.assembly()), &ops, false);
    let source = format!("unexpected_alias:\n{}", compiled.assembly());
    let mutated = assemble(&source);
    assert_eq!(mutated.bytes(), compiled.machine_code());
    check_layout(&mutated, &ops, false);
}

#[test]
#[should_panic(expected = "duplicate label names")]
fn label_inventory_rejects_duplicate_names() {
    let mut labels = vec![
        (NORMAL.to_owned(), ENTRY_PC),
        (OVERFLOW.to_owned(), ENTRY_PC + 28),
    ];
    let expected = labels.iter().map(|(name, _)| name.clone()).collect();
    check_label_inventory(&labels, &expected);
    labels[1] = labels[0].clone();
    check_label_inventory(&labels, &expected);
}

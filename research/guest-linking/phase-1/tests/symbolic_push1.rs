include!("support/symbolic_push1.rs");

fn assemble_pushes(immediates: &[u8]) -> (String, AssemblyResult) {
    assemble_push_guest(immediates, false)
}

#[test]
fn one_push1() {
    let (_, result) = assemble_pushes(&[2]);
    verify_push_guest(&result, &[2], false);
}

#[test]
fn two_push1_specimen() {
    let (source, result) = assemble_pushes(&[2, 3]);
    verify_push_guest(&result, &[2, 3], false);
    println!("Assembly:\n{source}");
    let code = result.bytes();
    print!("Bytes: ");
    for byte in code {
        print!("{byte:02x}");
    }
    println!();
    for (index, bytes) in code.as_chunks::<4>().0.iter().enumerate() {
        println!(
            "{:08x}: {:08x}",
            ENTRY + index as u64 * 4,
            u32::from_le_bytes(*bytes)
        );
    }
}

#[test]
fn repeated_push1_with_distinct_immediates() {
    let immediates = [0, 1, 127, 128, 255];
    let (_, result) = assemble_pushes(&immediates);
    verify_push_guest(&result, &immediates, false);
}

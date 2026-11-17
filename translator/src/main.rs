use ferrum_eth_translator::Opcode;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["translate", "--op", "ADD"] {
        let opcode = Opcode::from_byte(0x01).expect("ADD is opcode 0x01");
        print!("{}", opcode.assembly());
        ExitCode::SUCCESS
    } else {
        eprintln!("Usage: ferrum-eth-translator translate --op ADD");
        ExitCode::FAILURE
    }
}

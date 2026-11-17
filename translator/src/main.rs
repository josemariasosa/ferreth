use ferrum_eth_translator::Opcode;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, option, operation] if command == "translate" && option == "--op" => {
            let opcode = match operation.as_str() {
                "ADD" => Opcode::from_byte(0x01).expect("ADD is opcode 0x01"),
                "SUB" => Opcode::from_byte(0x03).expect("SUB is opcode 0x03"),
                _ => {
                    eprintln!("Usage: ferrum-eth-translator translate --op ADD|SUB");
                    return ExitCode::FAILURE;
                }
            };
            print!("{}", opcode.assembly());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("Usage: ferrum-eth-translator translate --op ADD|SUB");
            ExitCode::FAILURE
        }
    }
}

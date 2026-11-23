use std::io::{self, BufReader};
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdin = BufReader::new(io::stdin().lock());
    match miniq::protocol::serve(stdin, io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("miniq: {e}");
            ExitCode::FAILURE
        }
    }
}

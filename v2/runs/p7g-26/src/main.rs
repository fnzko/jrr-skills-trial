use std::io::{self, BufReader};
use std::process::ExitCode;

fn main() -> ExitCode {
    let input = BufReader::new(io::stdin().lock());
    match miniq::run(input, io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: {error}");
            ExitCode::FAILURE
        }
    }
}

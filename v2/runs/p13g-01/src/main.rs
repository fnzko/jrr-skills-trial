use std::io::{self, BufWriter};
use std::process::ExitCode;

fn main() -> ExitCode {
    let input = io::stdin().lock();
    let output = BufWriter::new(io::stdout().lock());
    match miniq::run(input, output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: {error}");
            ExitCode::FAILURE
        }
    }
}

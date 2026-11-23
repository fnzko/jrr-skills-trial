use std::io::{self, BufWriter};
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdin = io::stdin().lock();
    let stdout = BufWriter::new(io::stdout().lock());
    match miniq::run(stdin, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: {error}");
            ExitCode::FAILURE
        }
    }
}

use std::io::{self, BufWriter};
use std::process::ExitCode;

fn main() -> ExitCode {
    match miniq::serve(io::stdin().lock(), BufWriter::new(io::stdout().lock())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: {error}");
            ExitCode::FAILURE
        }
    }
}

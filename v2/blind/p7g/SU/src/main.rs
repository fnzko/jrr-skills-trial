use std::io::{self, BufWriter};
use std::process::ExitCode;

fn main() -> ExitCode {
    // The writer is flushed after every response, so buffering only batches the
    // small writes of one line.
    let output = BufWriter::new(io::stdout().lock());
    match miniq::run(io::stdin().lock(), output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: {error}");
            ExitCode::FAILURE
        }
    }
}

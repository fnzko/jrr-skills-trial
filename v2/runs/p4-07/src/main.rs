//! The `miniq` binary: reads commands on stdin and writes one response line per command.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    match miniq::protocol::serve(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: {error}");
            ExitCode::FAILURE
        }
    }
}

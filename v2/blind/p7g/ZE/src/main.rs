use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    match miniq::run(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: input/output failed: {error}");
            ExitCode::FAILURE
        }
    }
}

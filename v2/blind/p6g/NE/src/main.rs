use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    match miniq::serve::serve(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: input or output failed: {error}");
            ExitCode::FAILURE
        }
    }
}

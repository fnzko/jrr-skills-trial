use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    match miniq::serve(stdin.lock(), stdout.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("miniq: {err}");
            ExitCode::FAILURE
        }
    }
}

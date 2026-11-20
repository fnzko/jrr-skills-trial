use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    match miniq::run(stdin, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("miniq: {err}");
            ExitCode::FAILURE
        }
    }
}

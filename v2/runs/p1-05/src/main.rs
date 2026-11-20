use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    match miniq::session::serve(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("miniq: {e}");
            ExitCode::FAILURE
        }
    }
}

use std::io::{self, BufWriter};
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdin = io::stdin().lock();
    let stdout = BufWriter::new(io::stdout().lock());
    match miniq::server::serve(stdin, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("miniq: stdin/stdout failed: {err}");
            ExitCode::FAILURE
        }
    }
}

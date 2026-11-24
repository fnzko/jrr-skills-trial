use std::io::{self, BufWriter};
use std::process::ExitCode;

fn main() -> ExitCode {
    let input = io::stdin().lock();
    let output = BufWriter::new(io::stdout().lock());
    match miniq::serve(input, output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(cause) => {
            eprintln!("miniq: {cause}");
            ExitCode::FAILURE
        }
    }
}

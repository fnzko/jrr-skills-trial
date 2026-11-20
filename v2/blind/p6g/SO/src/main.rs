use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use miniq::Broker;

fn main() -> ExitCode {
    match run(io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("miniq: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Answers every input line with exactly one output line, flushing each
/// response before the next line is read.
fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut broker = Broker::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        // A final line without a terminator counts, as long as it is not empty
        // (which `read_until` already guarantees by returning a non-zero count).
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let response = miniq::respond(&mut broker, &line);
        output.write_all(response.as_bytes())?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

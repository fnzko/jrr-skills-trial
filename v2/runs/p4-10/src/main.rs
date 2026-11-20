use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut broker = miniq::Broker::new();
    match miniq::serve(io::stdin().lock(), io::stdout().lock(), &mut broker) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("miniq: {err}");
            ExitCode::FAILURE
        }
    }
}

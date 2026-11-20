//! The `miniq` binary: reads commands from stdin and writes responses to stdout
//! until end of input. Nothing else is printed, so stdout carries only protocol.

use std::io;

fn main() -> io::Result<()> {
    let mut broker = miniq::Broker::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    miniq::serve(&mut broker, stdin.lock(), stdout.lock())
}

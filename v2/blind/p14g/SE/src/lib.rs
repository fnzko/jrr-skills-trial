//! miniq: a small in-memory message queue driven by JSON commands, one per line.
//!
//! [`run`] reads commands from a reader and writes one response line per command; all state
//! lives in the [`store::Store`] it creates.

pub mod error;
pub mod fifo;
pub mod queue;
pub mod request;
pub mod response;
pub mod store;
pub mod types;

use std::io::{self, BufRead, Write};

use crate::error::Error;
use crate::request::Envelope;
use crate::response::{Reply, write_response};
use crate::store::Store;

/// Answers every line of `input` with one response line on `output`, until end of input.
///
/// A final line without `\n` counts when it is not empty. Only I/O failures end the loop early.
pub fn run(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut store = Store::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        write_response(&mut output, &handle(&mut store, &line))?;
    }
}

fn handle(store: &mut Store, line: &[u8]) -> Result<Reply, Error> {
    let envelope = Envelope::parse(line)?;
    store.execute(envelope.request()?)
}

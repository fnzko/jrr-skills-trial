//! miniq: an in-memory message queue driven by line-oriented JSON commands.
//!
//! The binary reads commands from stdin and writes one response per line to
//! stdout. All state lives in the process and time is virtual, so the same
//! input always produces the same output.

mod attributes;
mod broker;
mod command;
mod error;
mod protocol;
mod queue;

pub use error::{Error, ErrorCode};
pub use protocol::serve;

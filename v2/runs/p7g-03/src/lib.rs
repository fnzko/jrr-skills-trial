//! miniq: a small in-memory message queue driven by JSON commands.
//!
//! [`Service`] holds all state and turns one input line into one response
//! value; the binary only moves lines between stdin and stdout.

mod attributes;
mod error;
mod message_commands;
mod params;
mod queue;
mod queue_commands;
mod service;

pub use error::{Code, Error};
pub use service::Service;

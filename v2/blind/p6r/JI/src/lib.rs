//! miniq: a small in-memory message queue driven by JSON command lines.

mod attributes;
mod broker;
mod error;
mod ids;
mod naming;
mod queue;
mod request;
mod time;

pub use broker::Broker;
pub use error::{Error, ErrorCode, Result};

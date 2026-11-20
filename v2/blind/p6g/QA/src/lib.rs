//! miniq: a small in-memory message queue driven by JSON commands.
//!
//! Feed one line at a time to [`Broker::handle_line`]; it returns the
//! response line.

mod broker;
mod error;
mod ids;
mod queue;
mod request;
mod response;

pub use broker::Broker;
pub use error::{Error, ErrorCode};

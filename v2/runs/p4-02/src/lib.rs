//! miniq: a small in-memory message queue driven by JSON commands, one per line.
//!
//! The crate is layered so each part can be tested on its own:
//!
//! - [`request`] turns a command line into a typed [`request::Request`], doing every check that
//!   needs no queue state.
//! - [`queue`] stores one queue's messages and indexes them for cheap delivery and expiry.
//! - [`service`] holds all queues, the virtual clock and the id counters, and executes requests.
//! - [`protocol`] ties them together: one input line in, one JSON response line out.

pub mod error;
pub mod protocol;
pub mod queue;
pub mod request;
pub mod service;

pub use error::{Error, ErrorCode};
pub use protocol::handle_line;
pub use service::Service;

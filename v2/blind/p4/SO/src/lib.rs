//! miniq: an in-memory message queue driven by line-delimited JSON commands.
//!
//! The behaviour is specified in `SPEC.md` at the root of the repository.

pub mod attributes;
pub mod broker;
pub mod error;
pub mod params;
pub mod protocol;
pub mod queue;
pub mod syntax;

pub use broker::Broker;
pub use error::{Error, ErrorCode};
pub use protocol::serve;

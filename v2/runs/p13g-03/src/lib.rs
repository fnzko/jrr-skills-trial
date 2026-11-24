//! miniq: a small in-memory message queue driven by JSON lines.

pub mod command;
pub mod error;
pub mod params;
pub mod protocol;
pub mod queue;
pub mod reply;
pub mod service;
pub mod types;

pub use protocol::serve;
pub use service::Service;

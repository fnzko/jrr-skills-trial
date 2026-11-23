//! miniq: a small in-memory message queue driven by one JSON command per line.

mod attributes;
mod error;
mod params;
mod queue;
mod service;

pub use error::{Error, ErrorCode};
pub use service::Service;

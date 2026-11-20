//! miniq: a small in-memory message queue driven by JSON command lines.

pub mod error;
mod queue;
pub mod request;
mod response;
mod service;
pub mod types;

pub use service::Service;

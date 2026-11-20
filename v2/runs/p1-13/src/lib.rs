//! miniq: a small in-memory message queue driven by one JSON command per line.

pub mod command;
pub mod error;
pub mod ids;
pub mod output;
pub mod queue;
pub mod scalar;
pub mod server;
pub mod service;
pub mod text;

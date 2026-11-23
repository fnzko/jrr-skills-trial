//! miniq: a small in-memory message queue driven by JSON command lines.
//!
//! All state lives in the process and time is virtual: only `Tick` advances the clock.
//! [`server::serve`] runs the line protocol over any reader and writer.

pub mod attributes;
pub mod broker;
pub mod error;
pub mod queue;
pub mod request;
pub mod response;
pub mod server;
pub mod types;

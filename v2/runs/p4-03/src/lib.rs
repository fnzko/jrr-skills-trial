//! miniq: an in-memory message queue driven by JSON command lines. See SPEC.md.

pub mod attributes;
pub mod broker;
pub mod command;
pub mod error;
pub mod ids;
pub(crate) mod params;
pub mod queue;
pub mod reply;
pub mod time;

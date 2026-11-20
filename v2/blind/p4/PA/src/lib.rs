//! miniq: an in-memory message queue driven by line-based JSON commands.
//!
//! The protocol, error codes and state rules are specified in SPEC.md at the repository root.

#![forbid(unsafe_code)]

pub mod broker;
pub mod error;
pub mod protocol;
pub mod queue;
pub mod request;
pub mod response;
pub mod types;

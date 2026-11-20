//! miniq: an in-memory message queue spoken over a line-oriented JSON protocol.
//!
//! The crate is split so the protocol can be tested without a process:
//! [`Broker`] holds all state and executes parsed commands, and [`serve`]
//! runs the stdin/stdout loop on top of it.

pub mod attributes;
pub mod broker;
pub mod command;
pub mod error;
pub mod names;
pub mod params;
pub mod queue;
pub mod response;
pub mod session;

pub use broker::Broker;
pub use session::serve;

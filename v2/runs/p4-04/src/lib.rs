//! miniq: a small in-memory message queue with a virtual clock, specified in
//! SPEC.md. [`Broker::handle_line`] runs one protocol line; the `miniq`
//! binary feeds it stdin.

mod broker;
mod error;
mod queue;
mod request;
mod response;
mod types;

pub use broker::Broker;
pub use response::Response;

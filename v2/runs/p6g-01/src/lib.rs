//! miniq: a small in-memory message queue with virtual time.
//!
//! [`Broker`] is the whole service: it maps one command line to one response
//! line. The binary only moves lines between stdin, the broker and stdout.

mod broker;
mod error;
mod ids;
mod names;
mod queue;
mod request;
mod response;

pub use broker::Broker;

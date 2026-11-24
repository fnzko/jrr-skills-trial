//! Identifiers of messages, receipts and queue positions.
//!
//! Each kind has its own type so that one cannot be passed for another.
//! Message ids and receipt handles are counted by the broker over all queues.

use std::fmt;

/// `m-<n>` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns the current id and moves the counter to the next one.
    pub fn issue(&mut self) -> Self {
        let issued = *self;
        self.0 += 1;
        issued
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>` on the wire. Ordered by issue time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Returns the current handle and moves the counter to the next one.
    pub fn issue(&mut self) -> Self {
        let issued = *self;
        self.0 += 1;
        issued
    }

    /// Parses `r-<n>` exactly as it was issued: no sign, no leading zeros.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("r-")?;
        if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok().map(Self)
    }
}

impl fmt::Display for ReceiptHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r-{}", self.0)
    }
}

/// The position of a message in its queue; it only grows, so it orders
/// messages by arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Arrival(u64);

impl Arrival {
    pub const FIRST: Self = Self(0);

    pub fn issue(&mut self) -> Self {
        let issued = *self;
        self.0 += 1;
        issued
    }
}

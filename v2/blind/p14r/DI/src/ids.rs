//! Identifiers handed out by the broker and kept inside queues.

use std::fmt;

/// `m-<n>`: counts the accepted messages over all queues (SPEC section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns `self` and moves it on to the next identifier.
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

/// `r-<n>`: counts the deliveries over all queues (SPEC section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Returns `self` and moves it on to the next handle.
    pub fn issue(&mut self) -> Self {
        let issued = *self;
        self.0 += 1;
        issued
    }

    /// Parses `r-<n>` exactly as it is written: no sign, no leading zeros.
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

/// The position of a message in its queue's arrival order. It only grows, so
/// it stays valid when earlier messages leave.
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

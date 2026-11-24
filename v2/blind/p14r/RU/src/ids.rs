//! Identifiers that are all counters underneath but must not be mixed up.

use std::fmt;

/// `m-<n>`: counts accepted messages over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns this id and moves `self` to the one after it.
    pub fn take_next(&mut self) -> Self {
        let id = *self;
        self.0 += 1;
        id
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// `r-<n>`: counts deliveries over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Returns this handle and moves `self` to the one after it.
    pub fn take_next(&mut self) -> Self {
        let handle = *self;
        self.0 += 1;
        handle
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

/// Position of a message in its queue's arrival order; never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalKey(u64);

impl ArrivalKey {
    pub const FIRST: Self = Self(0);

    /// Returns this key and moves `self` to the one after it.
    pub fn take_next(&mut self) -> Self {
        let key = *self;
        self.0 += 1;
        key
    }
}

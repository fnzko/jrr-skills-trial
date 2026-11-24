//! Identifiers that clients see (`m-<n>`, `r-<n>`) and the queue's internal
//! arrival index, kept apart so that they cannot be mixed up.

use std::fmt;

/// The identifier of a message, shown as `m-<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(pub u64);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// A receipt handle, shown as `r-<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptHandle(pub u64);

impl ReceiptHandle {
    /// Parses `r-<n>` exactly as it is shown: no sign, no leading zeros.
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

/// The position of a message in its queue's arrival order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Arrival(pub u64);

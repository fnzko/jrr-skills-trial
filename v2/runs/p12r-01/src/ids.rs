//! Identifiers handed out by the broker and used inside a queue.
//!
//! They are separate types so that a receipt handle can never be used where
//! an arrival index or a message id is expected.

use std::fmt;

/// Position of a message in its queue; grows with every push and is never
/// reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArrivalIndex(u64);

impl ArrivalIndex {
    pub const FIRST: Self = Self(0);

    /// Returns the current index and moves to the next one.
    pub fn take_next(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
    }
}

/// Identifier of a message, `m-<n>` on the wire; unique over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageId(u64);

impl MessageId {
    pub const FIRST: Self = Self(1);

    /// Returns the current id and moves to the next one.
    pub fn take_next(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m-{}", self.0)
    }
}

/// Receipt handle, `r-<n>` on the wire; unique over all queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReceiptHandle(u64);

impl ReceiptHandle {
    pub const FIRST: Self = Self(1);

    /// Returns the current handle and moves to the next one.
    pub fn take_next(&mut self) -> Self {
        let current = *self;
        self.0 += 1;
        current
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_handles_parse_only_canonical_text() {
        assert_eq!(ReceiptHandle::parse("r-7"), Some(ReceiptHandle(7)));
        for bad in ["r-", "r-0", "r-07", "r--1", "r-+1", "x-1", "1", "r-1 "] {
            assert_eq!(ReceiptHandle::parse(bad), None, "{bad}");
        }
    }
}
